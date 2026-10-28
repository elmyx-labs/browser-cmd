//! Moving a tab: to an address, through its history, or reloading it.

use std::time::Duration;

use serde_json::json;

use crate::Result;
use crate::chrome::tab::Tab;

/// What came of asking a tab to go to an address.
pub(crate) enum Landing {
    Committed,
    /// The browser's reason, such as `net::ERR_NAME_NOT_RESOLVED`.
    Failed(String),
    /// Nothing answered within the wait, and the load was stopped.
    NoAnswer,
}

pub(crate) async fn navigate(tab: &Tab, url: &str, wait: Duration) -> Result<Landing> {
    let navigating = tab.call("Page.navigate", json!({ "url": url }));
    let Ok(answer) = tokio::time::timeout(wait, navigating).await else {
        // A site that never starts answering holds the tab until the network
        // gives up, minutes later: past the wait the load is stopped, so the
        // tab stays usable.
        let _ = tab.send("Page.stopLoading", json!({}));
        return Ok(Landing::NoAnswer);
    };
    Ok(match answer?["errorText"].as_str() {
        Some(reason) => Landing::Failed(reason.to_owned()),
        None => Landing::Committed,
    })
}

/// Go `step` entries back or forward; false when there is no entry there.
pub(crate) async fn history(tab: &Tab, step: i64) -> Result<bool> {
    let history = tab.call("Page.getNavigationHistory", json!({})).await?;
    let current = history["currentIndex"].as_i64().unwrap_or(0);
    let entries = history["entries"].as_array();
    let Some(entry) = entries.and_then(|entries| entries.get(usize::try_from(current + step).ok()?)) else {
        return Ok(false);
    };
    tab.call("Page.navigateToHistoryEntry", json!({ "entryId": entry["id"] })).await?;
    Ok(true)
}

pub(crate) async fn reload(tab: &Tab) -> Result<()> {
    tab.call("Page.reload", json!({})).await.map(drop)
}
