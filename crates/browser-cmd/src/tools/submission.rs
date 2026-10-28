//! The guard that refuses to send one form twice, on every road to a submission.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::js::SUBMITS;
use crate::session::Session;
use crate::{Error, Result};

/// How a form is about to be sent, for the guard that refuses to send one twice.
pub(crate) enum Submit {
    /// A click on a control — the node a ref named, or a point to hit-test.
    Click { node: Option<i64>, at: (f64, f64) },
    /// Enter in a field: the one named, or whatever has focus.
    Enter { node: Option<i64> },
    /// Space on whatever has focus, which presses a focused submit button.
    Space,
}

/// Refuse to send the same form twice while the page has not moved: a slow
/// page answers a click with nothing, and clicking again is how one order
/// becomes two. A new document clears it; `again` overrides it. Every road
/// to a submission passes here, because a guard on one road is a guard the
/// other roads walk around.
pub(crate) async fn guard_submission(
    session: &mut Session,
    tab: &Tab,
    how: Submit,
    again: bool,
) -> Result<()> {
    let (node, kind) = match how {
        Submit::Click { node: Some(node), .. } => (Some(node), "click"),
        Submit::Click { node: None, at } => match crate::page::dom::node_at(tab, at).await.ok().flatten() {
            Some(node) => (Some(node), "click"),
            None => return Ok(()),
        },
        Submit::Enter { node } => (node, "enter"),
        Submit::Space => (None, "click"),
    };
    let world = &mut session.document.world;
    let asked = match node {
        Some(node) => world.call_on(tab, node, SUBMITS, vec![json!(kind)]).await,
        None => world.call(tab, SUBMITS, vec![json!(kind)]).await,
    }
    .unwrap_or(Value::Null);
    if asked["submits"] != true {
        return Ok(());
    }
    let what = asked["id"].as_str().unwrap_or_default().to_owned();
    if !again && session.document.submitted.as_deref() == Some(what.as_str()) {
        return Err(Error::Tool(
            "this is the form you just submitted, and the page has not moved since. It may still be \
             working, or it may have failed quietly: wait, verify what changed, or read the page. \
             To send it a second time on purpose, repeat this with again=true."
                .into(),
        ));
    }
    // Reserved before dispatch, on purpose: if the browser never answers,
    // the form is still remembered as sent, and a retry is refused.
    session.document.submitted = Some(what);
    Ok(())
}
