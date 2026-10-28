//! The CLI's daemon client: one session named for the workspace, so a ref
//! taken by one command process names the same element in the next.

use std::path::Path;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::Outcome;
use crate::api::http::{self, ENDPOINT, Limits, SESSION_HEADER};
use crate::api::wire::Request;
use crate::{Error, Result};

pub(crate) struct Daemon {
    address: String,
    session: String,
    limits: Limits,
    token: Option<String>,
}

pub(crate) async fn up(address: &str, limits: &Limits) -> Result<bool> {
    http::endpoint(address)?;
    Ok(tokio::time::timeout(limits.io, tokio::net::TcpStream::connect(address))
        .await
        .is_ok_and(|result| result.is_ok()))
}

/// A session id fixed to the workspace the working directory is in, so each
/// project drives its own tab on the shared daemon from any of its
/// directories. `BROWSER_CMD_SESSION` names one instead, for two callers in
/// one workspace that must not share a tab.
fn session_id() -> String {
    match crate::env::var("SESSION") {
        Some(named) => format!("cli-{named}"),
        None => workspace_session(&std::env::current_dir().unwrap_or_default()),
    }
}

fn workspace_session(dir: &Path) -> String {
    let root = crate::workspace::root(dir);
    format!("cli-{:016x}", crate::workspace::digest(&root.to_string_lossy()))
}

/// Under `BROWSER_CMD_NO_SPAWN` the browser is someone else's to run: starting a
/// second one in a sandbox cannot work and elsewhere would split the session.
pub(crate) fn may_start_browser() -> Result<()> {
    match crate::env::flag("NO_SPAWN")? {
        Some(true) => Err(Error::Tool(
            "the browser for this session is not reachable and is not yours to start \
             (BROWSER_CMD_NO_SPAWN); retry once, then report that the browser is unavailable"
                .into(),
        )),
        _ => Ok(()),
    }
}

impl Daemon {
    pub(crate) fn attach(address: String) -> Result<Self> {
        Ok(Self { address, session: session_id(), limits: Limits::from_env()?, token: http::token()? })
    }

    /// Reach the daemon, starting `serve_with --serve ADDR` in the background if
    /// nothing answers. The child inherits this process's directory and
    /// environment, so it launches the same Chrome on the same profile.
    pub(crate) async fn connect(serve_with: &Path) -> Result<Self> {
        let daemon = Self::attach(http::address())?;
        if up(&daemon.address, &daemon.limits).await? {
            return Ok(daemon);
        }
        if !http::endpoint(&daemon.address).is_ok_and(|url| http::loopback(&url)) {
            return Err(Error::Tool("remote daemon is unavailable; start it on its host".into()));
        }
        may_start_browser()?;
        let mut child = std::process::Command::new(serve_with)
            .arg("--serve")
            .arg(&daemon.address)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| Error::Tool(format!("cannot start the browser daemon: {error}")))?;
        let ready = async {
            loop {
                if up(&daemon.address, &daemon.limits).await? {
                    return Ok(());
                }
                if let Some(status) = child.try_wait()? {
                    return Err(Error::Tool(format!("browser daemon exited during startup: {status}")));
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        };
        tokio::time::timeout(daemon.limits.io, ready)
            .await
            .map_err(|_| Error::Timeout("daemon startup"))??;
        Ok(daemon)
    }

    /// A transport failure is an error; a command that ran and failed is an
    /// outcome with `ok: false`.
    pub(crate) async fn call(&self, command: &str, args: Value) -> Result<Outcome> {
        let request = serde_json::to_string(&Request { command: command.to_owned(), args })?;
        tokio::time::timeout(self.limits.rpc, self.exchange(request)).await.map_err(|_| {
            Error::Timeout("daemon request; the operation may still be running, verify before retrying")
        })?
    }

    async fn exchange(&self, body: String) -> Result<Outcome> {
        http::endpoint(&self.address)?;
        if body.len() > self.limits.request_bytes {
            return Err(Error::Limit { part: "RPC request", limit: self.limits.request_bytes });
        }
        let authorization =
            self.token.as_ref().map(|token| format!("Authorization: Bearer {token}\r\n")).unwrap_or_default();
        let head = format!(
            "POST {ENDPOINT} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\n{SESSION_HEADER}: {}\r\n\
             {authorization}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.session,
            body.len(),
            address = self.address,
        );
        if head.len() > self.limits.header_bytes {
            return Err(Error::Limit { part: "RPC header", limit: self.limits.header_bytes });
        }
        let mut stream = tokio::net::TcpStream::connect(&self.address).await?;
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        let response =
            crate::http::read(&mut stream, self.limits.header_bytes, self.limits.response_bytes).await?;
        let mut status = response.start.split_whitespace();
        if !matches!(status.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
            return Err(Error::Protocol("daemon response is not HTTP".into()));
        }
        match status.next() {
            Some("200") => {}
            Some(code) => {
                return Err(Error::Protocol(format!(
                    "daemon HTTP {code}; operation outcome is not confirmed"
                )));
            }
            None => return Err(Error::Protocol("daemon response has no status".into())),
        }
        serde_json::from_slice(&response.body)
            .map_err(|error| Error::Protocol(format!("daemon response is not an outcome: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn reply(payload: Value) -> Result<Outcome> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let worker = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let asked = crate::http::read(&mut socket, 4096, 4096).await.unwrap();
            let body = payload.to_string();
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            asked.body
        });
        let result = Daemon::attach(address).unwrap().call("snapshot", json!({})).await;
        let asked: Value = serde_json::from_slice(&worker.await.unwrap()).unwrap();
        assert_eq!(asked["command"], "snapshot", "the daemon is asked for a command by name");
        result
    }

    #[test]
    fn every_directory_of_a_git_work_tree_is_one_session() {
        let temp = tempfile::tempdir().unwrap();
        let (project, deep) = (temp.path().join("project"), temp.path().join("project/src/deep"));
        std::fs::create_dir_all(&deep).unwrap();
        let apart = workspace_session(&deep);
        std::fs::create_dir(project.join(".git")).unwrap();
        assert_eq!(workspace_session(&deep), workspace_session(&project));
        assert_ne!(workspace_session(&deep), apart, "outside a work tree each directory is its own");
    }

    #[tokio::test]
    async fn a_malformed_answer_is_an_error_rather_than_an_empty_success() {
        for payload in [
            json!({"ok": true}),
            json!({"ok": true, "text": "page"}),
            json!({"text": "page", "images": []}),
            json!({"ok": "yes", "text": "page", "images": []}),
            json!({"ok": true, "text": "page", "images": [{"data": "x"}]}),
            json!([]),
        ] {
            assert!(reply(payload.clone()).await.is_err(), "{payload}");
        }
        let answered =
            reply(json!({"ok": false, "text": "Error: nothing here", "images": []})).await.unwrap();
        assert!(!answered.ok, "a command that ran and failed is an outcome, not a transport error");
        assert_eq!(answered.output, "Error: nothing here");
        assert!(!answered.unacknowledged());
        let unknown = reply(
            json!({"ok": false, "text": "Error: outcome unknown", "images": [], "error": "unacknowledged"}),
        )
        .await
        .unwrap();
        assert!(unknown.unacknowledged(), "the kind is read from the reply, not from its wording");
    }
}
