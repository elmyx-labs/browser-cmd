//! The daemon: one warm Chrome shared by any number of callers, each in a session of its own.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use tokio::io::AsyncWriteExt;

use crate::api::http::{self, ENDPOINT, Limits, SESSION_HEADER};
use crate::api::wire::Request;
use crate::session::Session;
use crate::{Error, Host, Outcome, Result};

type Sessions = RefCell<HashMap<String, Rc<tokio::sync::Mutex<Session>>>>;

/// How long one command may take, queueing included, before its caller is
/// told its outcome is unknown: under the 120 s a coding agent's shell gives
/// one command, so the caller hears that rather than being killed.
const COMMAND_DEADLINE: std::time::Duration = std::time::Duration::from_secs(100);

struct ServerState {
    host: Rc<Host>,
    sessions: Sessions,
    limits: Limits,
    token: Option<String>,
}

/// A caller names its session in `Browser-Session`; the first request under a
/// name opens it. Off loopback a token is required, and a foreign `Origin` is
/// refused, so a page in the very Chrome being driven cannot drive it.
pub(crate) async fn serve(
    listener: tokio::net::TcpListener,
    host: Rc<Host>,
    limits: Limits,
    token: Option<String>,
) -> Result<()> {
    if !listener.local_addr()?.ip().is_loopback() && token.is_none() {
        return Err(Error::Config("non-loopback --serve requires BROWSER_CMD_RPC_TOKEN".into()));
    }
    let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(limits.connections));
    eprintln!("browser-cmd: serving at http://{}{ENDPOINT}", listener.local_addr()?);
    let state = Rc::new(ServerState { host, sessions: RefCell::default(), limits, token });
    let signalled = terminate();
    tokio::pin!(signalled);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut stream, _) = accepted?;
                let Ok(permit) = capacity.clone().try_acquire_owned() else {
                    let refusal = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = send(&mut stream, refusal.into(), state.limits.io).await;
                    continue;
                };
                let state = state.clone();
                let started = tokio::time::Instant::now();
                tokio::task::spawn_local(async move {
                    let _permit = permit;
                    let _ = connection(stream, state, started).await;
                });
            }
            // A stop signal: close the browser gracefully so the warm Chrome
            // writes every session's cookies to its profile before it exits.
            _ = &mut signalled => {
                state.host.shutdown().await;
                let carried = crate::chrome::proxy::CARRIED.load(std::sync::atomic::Ordering::Relaxed);
                if carried > 0 {
                    eprintln!("browser-cmd: the proxy carried {carried} bytes");
                }
                return Ok(());
            }
        }
    }
}

async fn terminate() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(term) => term,
            Err(_) => return ctrl_c.await,
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}

async fn connection(
    mut stream: tokio::net::TcpStream,
    state: Rc<ServerState>,
    started: tokio::time::Instant,
) -> Result<()> {
    let reply = response(&mut stream, &state, started).await?;
    send(&mut stream, reply, state.limits.io).await
}

/// The request's checks, in order: its head within the deadline, the version,
/// authorization before any body is read, Host and Origin, the path, then the
/// body within the deadline and the method.
async fn response(
    stream: &mut tokio::net::TcpStream,
    state: &Rc<ServerState>,
    started: tokio::time::Instant,
) -> Result<String> {
    let local = stream.local_addr()?;
    let request_deadline = started + state.limits.rpc.min(COMMAND_DEADLINE);
    let deadline = (started + state.limits.io).min(request_deadline);
    let request = tokio::time::timeout_at(
        deadline,
        crate::http::read_head(stream, state.limits.header_bytes, state.limits.request_bytes),
    )
    .await;
    let mut request = match request {
        Ok(Ok(request)) => request,
        outcome => {
            let status = head_refusal(outcome.ok().and_then(Result::err).as_ref());
            return Ok(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
        }
    };
    let start = request.start.clone();
    let mut first = start.split_whitespace();
    let method = first.next().unwrap_or_default();
    let path = first.next().unwrap_or_default();
    let version = first.next().unwrap_or_default();
    let header = |name: &str| request.header(name);
    let refuse = |status: &str| Ok(respond(&state.limits, status, None, String::new()));
    if first.next().is_some() || version != "HTTP/1.1" {
        return refuse("400 Bad Request");
    }
    if !http::authorized(header("authorization"), state.token.as_deref()) {
        return refuse("401 Unauthorized");
    }
    let authority = header("host").and_then(|value| http::authority(value, local));
    if authority.is_none()
        || header("origin").is_some_and(|origin| !http::same_origin(origin, authority.as_ref().unwrap()))
    {
        return refuse("403 Forbidden");
    }
    if path != ENDPOINT {
        return refuse("404 Not Found");
    }
    let named = header(SESSION_HEADER).map(str::to_owned);
    // Authorize the bounded header before allocating or waiting for its body.
    match tokio::time::timeout_at(deadline, request.finish(stream)).await {
        Ok(Ok(())) => {}
        result => return refuse(if result.is_err() { "408 Request Timeout" } else { "400 Bad Request" }),
    }
    match method {
        "POST" => Ok(dispatch(state, &request.body, named, started, request_deadline).await),
        _ => refuse("405 Method Not Allowed"),
    }
}

/// A checked request's command, run in its session by the request deadline.
async fn dispatch(
    state: &Rc<ServerState>,
    body: &[u8],
    named: Option<String>,
    started: tokio::time::Instant,
    request_deadline: tokio::time::Instant,
) -> String {
    let refuse = |status: &str| respond(&state.limits, status, None, String::new());
    let Ok(asked) = serde_json::from_slice::<Request>(body) else {
        return refuse("400 Bad Request");
    };
    let Some(command) = crate::api::command::find(&asked.command) else {
        return refuse("404 Not Found");
    };
    // A client names its own session, so a ref taken by one command
    // process resolves in the next while that session is alive.
    let Some(id) = named else {
        return refuse("400 Bad Request");
    };
    // A request that waited behind the `close` ending its session opens a new one under the name.
    let finished = loop {
        let Some(session) = open(state, &id) else {
            return refuse("503 Service Unavailable");
        };
        // The command runs in a task of its own, so its caller can be
        // answered at the deadline while it finishes: dropping it
        // mid-key/gesture would not undo the browser-side effect.
        let (begun, began) = tokio::sync::oneshot::channel();
        let (done, finished) = tokio::sync::oneshot::channel();
        let worker = (state.clone(), id.clone(), asked.args.clone());
        tokio::task::spawn_local(async move {
            let (state, id, args) = worker;
            // A queued request that expires has not started its operation.
            let Ok(mut handler) = tokio::time::timeout_at(request_deadline, session.lock()).await else {
                return;
            };
            let current = state.sessions.borrow().get(&id).is_some_and(|now| Rc::ptr_eq(now, &session));
            // Sending fails once the caller has stopped waiting, so a
            // command never starts after it was refused.
            if begun.send(current).is_ok() && current {
                let acting = command.acts(&args);
                let outcome =
                    handler.perform(command.name, command.changing, acting, command.run, args).await;
                if handler.ended {
                    state.sessions.borrow_mut().remove(&id);
                }
                let _ = done.send(outcome);
            }
        });
        match tokio::time::timeout_at(request_deadline, began).await {
            Ok(Ok(true)) => break finished,
            Ok(Ok(false)) => {}
            _ => return refuse("503 Service Unavailable"),
        }
    };
    let outcome = match tokio::time::timeout_at(request_deadline, finished).await {
        Ok(Ok(outcome)) => outcome,
        _ => Outcome::failed(
            format!(
                "Error: outcome unknown: {} was still running at the daemon's {:.0} s deadline and \
                 goes on in the background. Look at the page before repeating it.",
                command.name,
                (request_deadline - started).as_secs_f64()
            ),
            "unacknowledged",
        ),
    };
    respond(&state.limits, "200 OK", Some(&id), serde_json::to_string(&outcome).unwrap_or_default())
}

/// The session a name holds, opened on its first request; none when every
/// slot is taken. A session idle past its time, and not waited on, is let go
/// with what it held.
fn open(state: &Rc<ServerState>, id: &str) -> Option<Rc<tokio::sync::Mutex<Session>>> {
    let mut sessions = state.sessions.borrow_mut();
    sessions.retain(|_, session| {
        Rc::strong_count(session) > 1
            || session.try_lock().is_ok_and(|session| session.used.elapsed() < state.limits.idle)
    });
    if let Some(session) = sessions.get(id) {
        return Some(session.clone());
    }
    if sessions.len() >= state.limits.sessions {
        return None;
    }
    let session = Rc::new(tokio::sync::Mutex::new(state.host.named_session(id)));
    sessions.insert(id.into(), session.clone());
    Some(session)
}

/// A reply with a JSON body, or an empty 500 for a body over the limit.
fn respond(limits: &Limits, status: &str, session: Option<&str>, body: String) -> String {
    let (status, body) = if body.len() > limits.response_bytes {
        ("500 Internal Server Error", String::new())
    } else {
        (status, body)
    };
    let extra = match status {
        "401 Unauthorized" => "WWW-Authenticate: Bearer realm=\"browser-cmd\"\r\n",
        "405 Method Not Allowed" => "Allow: POST\r\n",
        _ => "",
    };
    let session = session.map_or_else(String::new, |id| format!("{SESSION_HEADER}: {id}\r\n"));
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{session}{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

/// What a request whose head could not be read is answered with. `None` is a
/// head that never finished arriving; anything else is why it was refused.
fn head_refusal(why: Option<&Error>) -> &'static str {
    match why {
        None => "408 Request Timeout",
        Some(Error::Limit { part, .. }) if *part == crate::http::HEADER_PART => {
            "431 Request Header Fields Too Large"
        }
        Some(Error::Limit { .. }) => "413 Payload Too Large",
        Some(_) => "400 Bad Request",
    }
}

async fn send(stream: &mut tokio::net::TcpStream, reply: String, limit: std::time::Duration) -> Result<()> {
    tokio::time::timeout(limit, stream.write_all(reply.as_bytes()))
        .await
        .map_err(|_| Error::Timeout("HTTP response write"))??;
    Ok(())
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    use crate::http::{BODY_PART, HEADER_PART};

    /// The status must not turn on how either message happens to be worded.
    #[test]
    fn each_reason_a_head_is_refused_has_its_own_status() {
        assert!(head_refusal(None).starts_with("408"));
        assert!(head_refusal(Some(&Error::Limit { part: HEADER_PART, limit: 1 })).starts_with("431"));
        assert!(head_refusal(Some(&Error::Limit { part: BODY_PART, limit: 1 })).starts_with("413"));
        assert!(head_refusal(Some(&Error::Protocol("malformed".into()))).starts_with("400"));
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn state() -> (Rc<ServerState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = crate::Settings::new(temp.path(), temp.path());
        settings.headless = true;
        settings.pace = 0.0;
        let mut limits = Limits::from_env().unwrap();
        limits.io = std::time::Duration::from_millis(100);
        limits.rpc = std::time::Duration::from_millis(100);
        limits.request_bytes = 256;
        limits.sessions = 1;
        (
            Rc::new(ServerState {
                host: Host::new(settings),
                sessions: RefCell::default(),
                limits,
                token: None,
            }),
            temp,
        )
    }

    async fn exchange(state: Rc<ServerState>, raw: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let worker = tokio::task::spawn_local(connection(socket, state, tokio::time::Instant::now()));
        client.write_all(raw.replace("{addr}", &address.to_string()).as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(60), client.read_to_end(&mut reply))
            .await
            .unwrap()
            .unwrap();
        worker.await.unwrap().unwrap();
        String::from_utf8(reply).unwrap()
    }

    /// A request that opens or reaches a session without starting a browser.
    fn ask(session: &str) -> String {
        command(session, r#"{"command":"console","args":{}}"#)
    }

    fn command(session: &str, body: &str) -> String {
        format!(
            "POST {ENDPOINT} HTTP/1.1\r\nHost: {{addr}}\r\n{SESSION_HEADER}: {session}\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn headers_auth_and_limits_are_checked_before_waiting_for_a_body() {
        tokio::task::LocalSet::new().run_until(async {
            let (mut state, _temp) = state();
            for (raw, code) in [
                ("POST /call HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 99999999999\r\n\r\n", "413"),
                ("POST /call HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n", "400"),
                ("POST /call HTTP/1.1\r\nHost: localhost.evil.example\r\nContent-Length: 200\r\n\r\n", "403"),
                ("POST /call HTTP/1.1\r\nHost: {addr}\r\nOrigin: http://localhost:1\r\nContent-Length: 200\r\n\r\n", "403"),
                ("POST /call HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 200\r\n\r\n", "408"),
                ("POST /call HTTP/1.1\r\nHost: ", "408"),
            ] {
                let reply = exchange(state.clone(), raw).await;
                assert!(reply.starts_with(&format!("HTTP/1.1 {code}")), "{reply}");
            }
            Rc::get_mut(&mut state).unwrap().token = Some("fixture-token".into());
            let reply = exchange(state.clone(), "POST /call HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 200\r\n\r\n").await;
            assert!(reply.starts_with("HTTP/1.1 401"), "unauthorized request must not wait for a body: {reply}");
            let request = ask("one").replace("Host:", "Authorization: Bearer fixture-token\r\nHost:");
            assert!(exchange(state, &request).await.starts_with("HTTP/1.1 200"));
        }).await;
    }

    #[tokio::test]
    async fn unsupported_methods_keep_post_allow_and_zero_length_framing() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (state, _temp) = state();
                let raw = format!("HEAD {ENDPOINT} HTTP/1.1\r\nHost: {{addr}}\r\nContent-Length: 0\r\n\r\n");
                let reply = exchange(state, &raw).await;
                let (headers, body) = reply.split_once("\r\n\r\n").unwrap();
                assert!(headers.starts_with("HTTP/1.1 405 Method Not Allowed"), "{reply}");
                assert!(headers.contains("Allow: POST\r\n"), "{reply}");
                assert!(headers.contains("Content-Length: 0\r\n"), "{reply}");
                assert!(body.is_empty(), "{reply}");
            })
            .await;
    }

    #[tokio::test]
    async fn a_failed_command_names_the_kind_of_its_failure() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (state, _temp) = state();
                let body = r#"{"command":"upload","args":{"ref":"@e1.1","path":"x"}}"#;
                let raw = format!(
                    "POST {ENDPOINT} HTTP/1.1\r\nHost: {{addr}}\r\n{SESSION_HEADER}: s\r\n\
                     Content-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let reply = exchange(state, &raw).await;
                let json: serde_json::Value =
                    serde_json::from_str(reply.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                assert_eq!(json["ok"], false, "{reply}");
                assert_eq!(json["error"], "tool", "{reply}");
            })
            .await;
    }

    #[tokio::test]
    async fn sessions_are_bounded_and_a_busy_session_queues_until_the_deadline() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (state, _temp) = state();
                assert!(exchange(state.clone(), &ask("one")).await.starts_with("HTTP/1.1 200"));
                assert!(exchange(state.clone(), &ask("two")).await.starts_with("HTTP/1.1 503"));
                let session = state.sessions.borrow()["one"].clone();
                let held = session.lock().await;
                assert!(exchange(state.clone(), &ask("one")).await.starts_with("HTTP/1.1 503"));
                drop(held);
                assert!(exchange(state, &ask("one")).await.starts_with("HTTP/1.1 200"));
            })
            .await;
    }

    #[tokio::test]
    async fn a_request_queued_behind_close_opens_the_session_anew() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (mut state, _temp) = state();
                let limits = &mut Rc::get_mut(&mut state).unwrap().limits;
                (limits.rpc, limits.io) =
                    (std::time::Duration::from_secs(10), std::time::Duration::from_secs(10));
                assert!(exchange(state.clone(), &ask("one")).await.starts_with("HTTP/1.1 200"));
                let held = state.sessions.borrow()["one"].clone();
                let held = held.lock().await;
                let close = command("one", r#"{"command":"close","args":{}}"#);
                let closing = tokio::task::spawn_local({
                    let state = state.clone();
                    async move { exchange(state, &close).await }
                });
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let queued = tokio::task::spawn_local({
                    let state = state.clone();
                    async move { exchange(state, &ask("one")).await }
                });
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                drop(held);
                assert!(closing.await.unwrap().contains("The session ended"));
                let reply = queued.await.unwrap();
                assert!(reply.starts_with("HTTP/1.1 200") && reply.contains("No active tab"), "{reply}");
                assert!(state.sessions.borrow().contains_key("one"));
            })
            .await;
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome; run with --include-ignored"]
    async fn a_command_past_its_deadline_is_unknown_and_still_finishes() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (mut state, _temp) = state();
                let limits = &mut Rc::get_mut(&mut state).unwrap().limits;
                (limits.rpc, limits.io) =
                    (std::time::Duration::from_secs(1), std::time::Duration::from_secs(1));
                let slow = command("s", r#"{"command":"wait","args":{"seconds":3}}"#);
                let reply = exchange(state.clone(), &slow).await;
                assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
                assert!(
                    reply.contains(r#""error":"unacknowledged""#) && reply.contains("outcome unknown"),
                    "{reply}"
                );
                let finished = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                    while state.sessions.borrow()["s"].try_lock().is_err() {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                });
                assert!(finished.await.is_ok(), "the command did not finish in the background");
                assert!(exchange(state.clone(), &ask("s")).await.starts_with("HTTP/1.1 200"));
                state.host.shutdown().await;
            })
            .await;
    }

    /// What a command run in a session through the daemon answered.
    async fn said(state: &Rc<ServerState>, session: &str, body: serde_json::Value) -> String {
        let reply = exchange(state.clone(), &command(session, &body.to_string())).await;
        let body = reply.split("\r\n\r\n").nth(1).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(body)
            .map_or(reply.clone(), |json| json["text"].as_str().unwrap_or_default().to_owned())
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome; run with --include-ignored"]
    async fn close_ends_a_session_and_an_idle_one_is_let_go_each_freeing_its_slot_and_its_tabs() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (mut state, _temp) = state();
                let limits = &mut Rc::get_mut(&mut state).unwrap().limits;
                let (minute, idle) = (std::time::Duration::from_secs(60), std::time::Duration::from_secs(2));
                (limits.rpc, limits.io, limits.idle) = (minute, minute, idle);
                let tab =
                    |action: &str| serde_json::json!({ "command": "tab", "args": { "action": action } });
                let active = |name: &str| {
                    let sessions = state.sessions.borrow();
                    sessions[name].try_lock().unwrap().book.active.as_ref().unwrap().target.clone()
                };
                let opened = said(&state, "one", tab("new")).await;
                assert!(opened.contains("Opened tab t1"), "{opened}");
                let own = active("one");
                assert!(said(&state, "one", tab("new")).await.contains("Opened tab t2"));
                assert!(said(&state, "one", tab("keep")).await.contains("Kept t2"));
                let kept = active("one");
                let shared = state.host.shared().await.unwrap();
                let theirs =
                    shared.cdp.call(None, "Target.createTarget", serde_json::json!({ "url": "about:blank" }));
                let theirs = theirs.await.unwrap()["targetId"].as_str().unwrap().to_owned();
                let used = serde_json::json!({ "command": "tab", "args": { "action": "use", "id": theirs } });
                assert!(said(&state, "one", used).await.contains("Borrowed t3"));
                let ended = said(&state, "one", serde_json::json!({ "command": "close", "args": {} })).await;
                assert!(
                    ended.contains("Closed t1. Gave back t3, open. Left kept t2 open. The session ended"),
                    "{ended}"
                );
                assert!(state.sessions.borrow().is_empty(), "the slot is free");
                assert!(!shared.cdp.target_exists(&own).await.unwrap());
                assert!(
                    shared.cdp.target_exists(&kept).await.unwrap()
                        && shared.cdp.target_exists(&theirs).await.unwrap()
                );
                let listed = said(&state, "two", tab("list")).await;
                assert!(listed.contains("Not any session's") && listed.contains(&theirs[..8]), "{listed}");
                assert!(said(&state, "two", tab("new")).await.contains("Opened tab t1"));
                let idle = active("two");
                tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                let listed = said(&state, "three", tab("list")).await;
                assert!(
                    listed.contains(&idle[..8]) && listed.contains(&kept[..8]),
                    "let go with its tabs: {listed}"
                );
                assert!(!state.sessions.borrow().contains_key("two"));
                state.host.shutdown().await;
            })
            .await;
    }

    #[tokio::test]
    async fn accepted_connections_are_bounded() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (state, _temp) = state();
                let mut limits = state.limits.clone();
                limits.connections = 2;
                limits.io = std::time::Duration::from_secs(2);
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let host = state.host.clone();
                tokio::task::spawn_local(serve(listener, host, limits, None));
                let _first = tokio::net::TcpStream::connect(address).await.unwrap();
                let _second = tokio::net::TcpStream::connect(address).await.unwrap();
                let mut third = tokio::net::TcpStream::connect(address).await.unwrap();
                let mut reply = String::new();
                tokio::time::timeout(std::time::Duration::from_secs(1), third.read_to_string(&mut reply))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(reply.starts_with("HTTP/1.1 503"), "excess connection was not refused: {reply}");
            })
            .await;
    }
}
