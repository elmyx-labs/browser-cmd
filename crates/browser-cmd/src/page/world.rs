//! Where our JavaScript runs: an isolated world that shares the page's DOM but
//! none of its globals, so the page cannot see or wrap what we call, made with
//! `Page.createIsolatedWorld` so it never gets the `Runtime.enable` side
//! effects that Cloudflare and DataDome test for.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::frames::{Epoch, FrameSet};
use crate::{Error, Result};

/// How much of a page script's own error message is carried back: enough to
/// name what threw, without a minified stack trace behind it.
const SCRIPT_ERROR_CHARS: usize = 400;

/// The isolated world's name, which the page never sees.
const NAME: &str = "util";

/// One isolated world per document, made on first use and again after the
/// document it belonged to is gone.
pub(crate) struct World {
    context: Option<(i64, Epoch)>,
    name: String,
}

pub(crate) struct NodeResult {
    pub node: Option<i64>,
    pub detail: Option<String>,
}

/// Inspector objects are scoped to one call and released on every exit,
/// including cancellation. Their group names are not DOM attributes or globals.
struct Objects {
    tab: Tab,
    name: String,
}

impl Objects {
    fn new(tab: &Tab) -> Self {
        Self { tab: tab.clone(), name: format!("browser-cmd-objects-{}", unique()) }
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        let _ = self.tab.send("Runtime.releaseObjectGroup", json!({ "objectGroup": self.name }));
    }
}

/// A number no other call in this process has been given.
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl World {
    /// The world internal helpers run in.
    pub(crate) fn new() -> Self {
        Self { context: None, name: NAME.to_owned() }
    }

    /// A world for a caller's own scripts, whose globals stay apart from the
    /// helpers' and from any other session's.
    pub(crate) fn caller() -> Self {
        let time =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        Self { context: None, name: format!("{NAME}:caller:{}:{time:x}:{}", std::process::id(), unique()) }
    }

    pub(crate) fn reset(&mut self) {
        self.context = None;
    }

    async fn context(&mut self, tab: &Tab) -> Result<i64> {
        if let Some((context, captured)) = self.context
            && captured == Epoch::of(tab)
        {
            return Ok(context);
        }
        let FrameSet { tree, epoch } = FrameSet::capture(tab).await?;
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or_else(|| Error::Tool("the page has no frame".into()))?;
        let context = self.context_in(tab, frame).await?;
        epoch.check(tab).inspect_err(|_| self.reset())?;
        self.context = Some((context, epoch));
        Ok(context)
    }

    /// A world of this name in one frame.
    pub(crate) async fn context_in(&self, tab: &Tab, frame: &str) -> Result<i64> {
        let created =
            tab.call("Page.createIsolatedWorld", json!({ "frameId": frame, "worldName": self.name })).await?;
        created["executionContextId"]
            .as_i64()
            .ok_or_else(|| Error::Tool("the browser made no isolated world".into()))
    }

    /// Evaluate a caller's expression exactly once. A script may have changed
    /// the page before failing; its error is never a license to replay it.
    pub(crate) async fn evaluate(&mut self, tab: &Tab, expression: &str) -> Result<Value> {
        let context = self.context(tab).await?;
        let group = Objects::new(tab);
        let params = json!({
            "expression": expression, "contextId": context, "objectGroup": group.name,
            "returnByValue": true, "awaitPromise": true,
        });
        let answer = tab.call("Runtime.evaluate", params).await;
        self.answered(tab, context, answer)
    }

    /// Call one of our functions with arguments, `this` being the document's global.
    pub(crate) async fn call(&mut self, tab: &Tab, function: &str, arguments: Vec<Value>) -> Result<Value> {
        let context = self.context(tab).await?;
        let answer = call_in(tab, context, function, arguments).await;
        self.answered(tab, context, answer)
    }

    /// Call a function with a DOM node as `this`.
    pub(crate) async fn call_on(
        &mut self,
        tab: &Tab,
        node: i64,
        function: &str,
        arguments: Vec<Value>,
    ) -> Result<Value> {
        let context = self.context(tab).await?;
        let group = Objects::new(tab);
        let object = resolve(tab, context, node, &group).await?;
        let answer = tab
            .call(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object, "functionDeclaration": function, "arguments": boxed(arguments),
                    "returnByValue": true, "awaitPromise": true,
                }),
            )
            .await;
        self.answered(tab, context, answer)
    }

    /// Run an internal node query returning null or `{node, detail}`, on a node
    /// or on the document. Only the found DOM object is resolved; nothing is
    /// marked in the document.
    pub(crate) async fn node_result(
        &mut self,
        tab: &Tab,
        target: Option<i64>,
        function: &str,
        arguments: Vec<Value>,
    ) -> Result<NodeResult> {
        let context = self.context(tab).await?;
        let group = Objects::new(tab);
        let mut params = json!({
            "functionDeclaration": function, "returnByValue": false, "awaitPromise": true,
            "objectGroup": group.name, "arguments": boxed(arguments),
        });
        match target {
            Some(node) => params["objectId"] = json!(resolve(tab, context, node, &group).await?),
            None => params["executionContextId"] = json!(context),
        }
        let response = tab.call("Runtime.callFunctionOn", params).await?;
        check_exception(&response)?;
        if response["result"]["subtype"] == "null" {
            return Ok(NodeResult { node: None, detail: None });
        }
        let object = response["result"]["objectId"]
            .as_str()
            .ok_or_else(|| Error::Tool("node query returned no object".into()))?;
        let properties =
            tab.call("Runtime.getProperties", json!({ "objectId": object, "ownProperties": true })).await?;
        check_exception(&properties)?;
        let property = |name: &str| {
            properties["result"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"] == name)
                .map(|p| &p["value"])
        };
        let node = property("node").ok_or_else(|| Error::Tool("node query omitted its node field".into()))?;
        let detail = property("detail").and_then(|value| value["value"].as_str()).map(str::to_owned);
        let node = match node["subtype"] == "null" {
            true => None,
            false => {
                let object = node["objectId"]
                    .as_str()
                    .ok_or_else(|| Error::Tool("node query returned an invalid node".into()))?;
                let described =
                    tab.call("DOM.describeNode", json!({ "objectId": object, "depth": 0 })).await?;
                Some(
                    described["node"]["backendNodeId"]
                        .as_i64()
                        .ok_or_else(|| Error::Tool("node query has no backend identity".into()))?,
                )
            }
        };
        Ok(NodeResult { node, detail })
    }

    /// A failed call leaves the world suspect, and one made across a navigation
    /// is reported as that rather than as the script's failure.
    fn answered(&mut self, tab: &Tab, context: i64, answer: Result<Value>) -> Result<Value> {
        match answer {
            Ok(value) => unwrap_result(value),
            Err(error) => {
                let changed =
                    self.context.is_some_and(|(held, epoch)| held == context && epoch != Epoch::of(tab));
                self.reset();
                Err(if changed { Error::DocumentChanged } else { error })
            }
        }
    }
}

/// Whether the page answers at all within `wait`: a hung renderer does not.
pub(crate) async fn answers(tab: &Tab, wait: std::time::Duration) -> bool {
    let probe = tab.call("Runtime.evaluate", json!({ "expression": "1", "returnByValue": true }));
    tokio::time::timeout(wait, probe).await.is_ok_and(|answer| answer.is_ok())
}

/// Call a function in one execution context, by value.
pub(crate) async fn call_in(tab: &Tab, context: i64, function: &str, arguments: Vec<Value>) -> Result<Value> {
    let group = Objects::new(tab);
    let params = json!({
        "functionDeclaration": function, "executionContextId": context, "arguments": boxed(arguments),
        "objectGroup": group.name, "returnByValue": true, "awaitPromise": true,
    });
    tab.call("Runtime.callFunctionOn", params).await
}

async fn resolve(tab: &Tab, context: i64, node: i64, group: &Objects) -> Result<String> {
    let gone = || {
        Error::Tool(
            "the element is no longer on the page — the page re-rendered; take a fresh snapshot".into(),
        )
    };
    let resolved = tab
        .call(
            "DOM.resolveNode",
            json!({ "backendNodeId": node, "executionContextId": context, "objectGroup": group.name }),
        )
        .await
        .map_err(|error| match error {
            Error::Cdp(_) => gone(),
            _ => error,
        })?;
    resolved["object"]["objectId"].as_str().map(str::to_owned).ok_or_else(gone)
}

/// Arguments as the `Runtime` domain wants them: each value in a box of its own.
fn boxed(arguments: Vec<Value>) -> Vec<Value> {
    arguments.into_iter().map(|value| json!({ "value": value })).collect()
}

pub(crate) fn unwrap_result(response: Value) -> Result<Value> {
    check_exception(&response)?;
    Ok(response["result"]["value"].clone())
}

fn check_exception(response: &Value) -> Result<()> {
    if let Some(exception) = response.get("exceptionDetails") {
        let text =
            exception["exception"]["description"].as_str().or(exception["text"].as_str()).unwrap_or("error");
        return Err(Error::Tool(format!(
            "page script failed: {}",
            crate::text::clip(text, SCRIPT_ERROR_CHARS)
        )));
    }
    Ok(())
}
