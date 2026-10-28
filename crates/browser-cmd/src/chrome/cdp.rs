//! The DevTools Protocol, as little of it as driving a browser needs.
//!
//! A CDP client is a WebSocket, a counter, and a map from the ids of the
//! commands in flight to whoever is waiting for their answers. Everything else
//! the big clients add — typed bindings, frame managers, event listeners — is
//! also where they hand the page a way to notice them: every one of them
//! enables the Runtime domain on attach, which is the leak Cloudflare and
//! DataDome test for. Owning the event subscriptions here avoids that.
//!
//! Sessions are flattened: a tab is attached with `Target.attachToTarget` and
//! its session id rides on every command sent to it.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message;

use crate::chrome::events::Events;
use crate::settings::Dialogs;
use crate::{Error, Result};

/// How often a closed resource is asked after again, while waiting for the
/// browser to agree that it is gone.
const CLOSE_POLL: Duration = Duration::from_millis(50);

/// How long the browser gets to answer a liveness question after a command
/// went unanswered.
const PROBE: Duration = Duration::from_secs(5);

struct Waiter {
    answer: oneshot::Sender<Result<Value>>,
    attaching: bool,
}

type Waiting = Arc<Mutex<HashMap<u64, Waiter>>>;

/// A caller may drop its future before the CDP timeout expires. Reclaim the
/// waiter even then; this does not pretend to undo a browser-side operation.
struct Pending<'a> {
    waiting: &'a Waiting,
    id: u64,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.waiting.lock().unwrap().remove(&self.id);
    }
}

pub(crate) struct Cdp {
    endpoint: String,
    out: mpsc::UnboundedSender<String>,
    waiting: Waiting,
    next: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    timeout: Duration,
    events: Arc<Mutex<Events>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Cdp {
    /// Connect, and answer every dialog a page opens from then on as `dialogs`
    /// says, at once, because a dialog left standing stops its tab.
    pub(crate) async fn connect(url: &str, timeout: Duration, dialogs: Dialogs) -> Result<Self> {
        let (socket, _) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(url))
            .await
            .map_err(|_| Error::Tool(format!("connecting to {url} timed out")))?
            .map_err(|error| Error::Tool(format!("cannot connect to {url}: {error}")))?;
        let (mut sink, mut stream) = socket.split();
        let (out, mut queue) = mpsc::unbounded_channel::<String>();
        let waiting: Waiting = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let next = Arc::new(AtomicU64::new(1));
        let events: Arc<Mutex<Events>> = Arc::default();

        let (unsent, down) = (waiting.clone(), alive.clone());
        let writer = tokio::spawn(async move {
            while let Some(line) = queue.recv().await {
                if sink.send(Message::Text(line.into())).await.is_err() {
                    // Nothing more reaches the browser: fail every caller now
                    // instead of letting each wait out its timeout.
                    down.store(false, Ordering::Relaxed);
                    unsent.lock().unwrap().clear();
                    break;
                }
            }
        });

        let (answers, gone, counter, sender) = (waiting.clone(), alive.clone(), next.clone(), out.clone());
        let recorded = events.clone();
        let reader = tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(message) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(mut answer) = recorded.lock().unwrap().apply(&message, dialogs) {
                    answer["id"] = json!(counter.fetch_add(1, Ordering::Relaxed));
                    let _ = sender.send(answer.to_string());
                }
                let Some(id) = message["id"].as_u64() else { continue };
                if let Some(waiter) = answers.lock().unwrap().remove(&id) {
                    let answer = match message.get("error") {
                        Some(error) => {
                            Err(Error::Cdp(error["message"].as_str().unwrap_or("command failed").into()))
                        }
                        None => Ok(message["result"].clone()),
                    };
                    // Record before waking the caller: a detach event can follow
                    // the response before the attaching future gets polled again.
                    if waiter.attaching
                        && let Ok(result) = &answer
                        && let Some(session) = result["sessionId"].as_str().filter(|id| !id.is_empty())
                    {
                        recorded.lock().unwrap().attachments.insert(session.to_owned());
                    }
                    let _ = waiter.answer.send(answer);
                }
            }
            // The socket closed: nothing in flight will ever be answered.
            gone.store(false, Ordering::Relaxed);
            answers.lock().unwrap().clear();
        });

        Ok(Self {
            endpoint: url.into(),
            out,
            waiting,
            next,
            alive,
            timeout,
            events,
            tasks: vec![writer, reader],
        })
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// What the browser volunteered; never held across an `.await`.
    pub(crate) fn events(&self) -> MutexGuard<'_, Events> {
        self.events.lock().unwrap()
    }

    /// Let go of an attached session. Releasing an observer must not close the page.
    pub(crate) fn detach(&self, session: &str, target: &str) {
        self.events().release(session, target);
        let _ = self.send(None, "Target.detachFromTarget", json!({ "sessionId": session }));
    }

    /// Have the browser tell this connection of every page's creation, change, crash and end.
    pub(crate) async fn discover(&self) -> Result<()> {
        self.call(None, "Target.setDiscoverTargets", json!({ "discover": true })).await.map(drop)
    }

    /// The browser's own user agent.
    pub(crate) async fn user_agent(&self) -> Result<String> {
        let version = self.call(None, "Browser.getVersion", json!({})).await?;
        let agent = version["userAgent"].as_str().map(str::to_owned);
        agent.ok_or_else(|| Error::Tool("browser has no user agent".into()))
    }

    /// Every target the browser has, of whatever type, each with its identity.
    pub(crate) async fn targets(&self) -> Result<Vec<Value>> {
        // Do not inherit a caller's discovery filter: it could hide a live target.
        let reply = self.call(None, "Target.getTargets", json!({ "filter": [{}] })).await?;
        let targets = reply["targetInfos"]
            .as_array()
            .ok_or_else(|| Error::Protocol("browser returned no target list".into()))?;
        if targets.iter().any(|target| target["targetId"].as_str().is_none_or(str::is_empty)) {
            return Err(Error::Protocol("browser returned a target without identity".into()));
        }
        Ok(targets.clone())
    }

    /// The browser contexts created in it, and the default one's id when it gives it.
    pub(crate) async fn contexts(&self) -> Result<(Vec<String>, Option<String>)> {
        let reply = self.call(None, "Target.getBrowserContexts", json!({})).await?;
        let ids = reply["browserContextIds"].as_array().and_then(|ids| {
            ids.iter().map(|id| id.as_str().filter(|id| !id.is_empty()).map(str::to_owned)).collect()
        });
        let ids = ids.ok_or_else(|| Error::Protocol("browser returned no valid context list".into()))?;
        Ok((ids, reply["defaultBrowserContextId"].as_str().map(str::to_owned)))
    }

    /// A new browser context's identity; none when the browser gave none.
    pub(crate) async fn create_browser_context(&self) -> Result<Option<String>> {
        let created = self.call(None, "Target.createBrowserContext", json!({})).await?;
        Ok(created["browserContextId"].as_str().filter(|id| !id.is_empty()).map(str::to_owned))
    }

    pub(crate) async fn context_exists(&self, id: &str) -> Result<bool> {
        Resource::Context(id).present(self).await
    }

    pub(crate) async fn target_exists(&self, id: &str) -> Result<bool> {
        Resource::Target(id).present(self).await
    }

    pub(crate) async fn close_target(&self, target: &str) -> Result<()> {
        self.close(Resource::Target(target)).await
    }

    pub(crate) async fn dispose_browser_context(&self, context: &str) -> Result<()> {
        self.close(Resource::Context(context)).await
    }

    /// Close a target without waiting to hear it closed, for a cleanup that cannot wait.
    pub(crate) fn abandon_target(&self, target: &str) {
        self.abandon(Resource::Target(target));
    }

    /// Dispose a context without waiting to hear it gone, for a cleanup that cannot wait.
    pub(crate) fn abandon_browser_context(&self, context: &str) {
        self.abandon(Resource::Context(context));
    }

    fn abandon(&self, resource: Resource<'_>) {
        let (method, params) = resource.close_request();
        let _ = self.send(None, method, params);
    }

    /// Close or dispose a resource and confirm it is gone. Both are single
    /// browser commands; if the response is lost, read-only state establishes
    /// absence without replaying them.
    async fn close(&self, resource: Resource<'_>) -> Result<()> {
        if resource.id().is_empty() {
            return Err(Error::Tool("cannot close a resource without its identity".into()));
        }
        tokio::time::timeout(self.timeout, async {
            let (method, params) = resource.close_request();
            let failure = match self.call(None, method, params).await {
                Ok(reply)
                    if reply.is_object()
                        && (!matches!(resource, Resource::Target(_)) || reply["success"] == true) =>
                {
                    None
                }
                Ok(_) => Some(Error::Protocol("resource closure was not acknowledged by Chrome".into())),
                Err(error) => Some(error),
            };
            loop {
                if !resource.present(self).await? {
                    return Ok(());
                }
                if let Some(error) = failure {
                    return Err(Error::Tool(format!("resource closure was not confirmed: {error}")));
                }
                tokio::time::sleep(CLOSE_POLL).await;
            }
        })
        .await
        .map_err(|_| Error::Timeout("browser resource cleanup; outcome remains unconfirmed"))?
    }

    fn message(&self, session: Option<&str>, method: &str, params: Value) -> (u64, Value) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_owned());
        }
        (id, message)
    }

    /// Send one command, to the browser or to one attached tab, and wait for its answer.
    ///
    /// A command left unanswered may be a busy page or a dead wire. The browser
    /// itself is asked something cheap: if it does not answer either, the
    /// connection is given up, so the next command reconnects and re-attaches
    /// instead of waiting out every timeout on it. Nothing is replayed.
    pub(crate) async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        let answer = self.request(session, method, params, self.timeout).await;
        if matches!(answer, Err(Error::Unacknowledged(_)))
            && self.request(None, "Browser.getVersion", json!({}), PROBE.min(self.timeout)).await.is_err()
        {
            self.disconnect();
        }
        answer
    }

    async fn request(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if !self.alive() {
            return Err(Error::Closed);
        }
        let (id, message) = self.message(session, method, params);
        let (answer, answered) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap()
            .insert(id, Waiter { answer, attaching: method == "Target.attachToTarget" });
        let _pending = Pending { waiting: &self.waiting, id };
        if self.out.send(message.to_string()).is_err() {
            return Err(Error::Closed);
        }
        // Past this point the request is on the wire: a lost reply is not a
        // request that failed, it is one whose effect is unknown.
        match tokio::time::timeout(timeout, answered).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) | Err(_) => Err(Error::Unacknowledged(method.to_owned())),
        }
    }

    /// Queue a command without waiting for its answer. This acknowledges only
    /// local enqueueing, not delivery or an effect in Chrome.
    pub(crate) fn send(&self, session: Option<&str>, method: &str, params: Value) -> Result<()> {
        if !self.alive() {
            return Err(Error::Closed);
        }
        let (_, message) = self.message(session, method, params);
        self.out.send(message.to_string()).map_err(|_| Error::Closed)
    }

    /// A client with no browser behind it. The receiving end of its outgoing
    /// queue comes back, so a test can read what was sent or close the wire.
    #[cfg(test)]
    pub(crate) fn fixture(alive: bool, timeout: Duration) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (out, queue) = mpsc::unbounded_channel();
        let client = Self {
            endpoint: String::new(),
            out,
            waiting: Arc::default(),
            next: Arc::new(AtomicU64::new(1)),
            alive: Arc::new(AtomicBool::new(alive)),
            timeout,
            events: Arc::default(),
            tasks: Vec::new(),
        };
        (client, queue)
    }

    /// Stop talking to the browser, as a dropped socket would.
    pub(crate) fn disconnect(&self) {
        for task in &self.tasks {
            task.abort();
        }
        self.alive.store(false, Ordering::Relaxed);
        self.waiting.lock().unwrap().clear();
    }
}

impl Drop for Cdp {
    fn drop(&mut self) {
        self.disconnect();
    }
}

enum Resource<'a> {
    Target(&'a str),
    Context(&'a str),
}

impl Resource<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Target(id) | Self::Context(id) => id,
        }
    }

    fn close_request(&self) -> (&'static str, Value) {
        match self {
            Self::Target(id) => ("Target.closeTarget", json!({ "targetId": id })),
            Self::Context(id) => ("Target.disposeBrowserContext", json!({ "browserContextId": id })),
        }
    }

    async fn present(&self, cdp: &Cdp) -> Result<bool> {
        Ok(match self {
            Self::Target(id) => cdp.targets().await?.iter().any(|target| target["targetId"] == *id),
            Self::Context(id) => cdp.contexts().await?.0.iter().any(|context| context == id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_sent_request_with_no_reply_is_unacknowledged_not_failed() {
        let (cdp, receiver) = Cdp::fixture(true, Duration::from_millis(50));
        let lost = cdp.call(None, "Input.dispatchMouseEvent", json!({})).await;
        assert!(
            matches!(lost, Err(Error::Unacknowledged(ref method)) if method == "Input.dispatchMouseEvent"),
            "{lost:?}"
        );
        assert!(lost.unwrap_err().to_string().contains("may have taken effect"));
        assert!(!cdp.alive(), "a browser that answers nothing either is given up for a reconnect");
        // With nobody to send to, nothing reached the wire: that is a plain failure.
        drop(receiver);
        let dead = cdp.call(None, "Input.dispatchMouseEvent", json!({})).await;
        assert!(matches!(dead, Err(Error::Closed)), "{dead:?}");
    }

    #[tokio::test]
    async fn an_unanswered_page_command_keeps_a_browser_that_still_answers() {
        let (client, mut queue) = Cdp::fixture(true, Duration::from_millis(50));
        let cdp = Arc::new(client);
        let answering = cdp.clone();
        let browser = tokio::spawn(async move {
            while let Some(line) = queue.recv().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                if message["method"] == "Browser.getVersion" {
                    let id = message["id"].as_u64().unwrap();
                    let waiter = answering.waiting.lock().unwrap().remove(&id).unwrap();
                    let _ = waiter.answer.send(Ok(json!({})));
                }
            }
        });
        let lost = cdp.call(Some("busy"), "Runtime.evaluate", json!({})).await;
        assert!(matches!(lost, Err(Error::Unacknowledged(_))), "{lost:?}");
        assert!(cdp.alive());
        browser.abort();
    }

    #[tokio::test]
    async fn a_cancelled_call_releases_its_waiter_without_replaying_the_command() {
        let (client, mut queue) = Cdp::fixture(true, Duration::from_secs(60));
        let waiting = client.waiting.clone();
        let cdp = Arc::new(client);
        let call =
            tokio::spawn(
                async move { cdp.call(None, "Runtime.evaluate", json!({"expression":"pending"})).await },
            );
        assert!(queue.recv().await.unwrap().contains("Runtime.evaluate"));
        assert_eq!(waiting.lock().unwrap().len(), 1);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(waiting.lock().unwrap().is_empty());
        assert!(queue.try_recv().is_err());
    }
}
