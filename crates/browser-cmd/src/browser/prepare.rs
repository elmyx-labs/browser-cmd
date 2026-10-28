//! What every tab is told before a session uses it.

use serde_json::json;

use crate::browser::host::Shared;
use crate::browser::identity::{Identity, apply_identity};
use crate::chrome::tab::{Tab, watch_frames};
use crate::{Error, Result, Settings};

impl Shared {
    /// A page attached to, then prepared.
    pub(crate) async fn attach(&self, settings: &Settings, target: String) -> Result<Tab> {
        let tab = crate::chrome::tab::attach(&self.cdp, target).await?;
        prepare(settings, &tab, self.identity.as_ref()).await?;
        Ok(tab)
    }
}

/// What every new tab is told before it loads anything.
pub(crate) async fn prepare(settings: &Settings, tab: &Tab, identity: Option<&Identity>) -> Result<()> {
    tab.call("Accessibility.enable", json!({})).await?;
    watch_frames(tab).await?;
    // The browser's own log — errors, blocked requests, CSP — for `console` to
    // read back. The Log domain does not create the Runtime execution-context
    // side effect that bot walls watch for, so the page sees an ordinary browser.
    let _ = tab.call("Log.enable", json!({})).await;
    // A crashed page is said, not met as a silence. Like Log, the Inspector
    // domain leaves nothing a script on the page can see.
    let _ = tab.call("Inspector.enable", json!({})).await;
    let asked = tab.cdp.events().traffic.asked.contains(&tab.target);
    if settings.network || settings.network_notes || asked {
        crate::chrome::traffic::record(tab, settings.network_notes).await?;
    }
    if settings.canvas_text {
        crate::page::canvas::record(tab).await?;
    }
    if settings.attach.is_some() {
        // A Chrome we only attached to is somebody's; it keeps its own ways.
        return Ok(());
    }
    if settings.headless {
        let identity =
            identity.ok_or_else(|| Error::Config("native browser identity was not initialized".into()))?;
        apply_identity(tab, identity).await?;
    }
    // A download lands where the workspace can read it, instead of in a
    // prompt nobody will answer.
    std::fs::create_dir_all(&settings.downloads)?;
    // With events, so that a click which answers with a file is reported as
    // one rather than as nothing having happened. The Browser domain carries
    // them, not the Network domain.
    // Browser commands use an omitted id for the default context, even when
    // TargetInfo reports its internal id. Query created contexts first: this
    // already-existing target cannot acquire a newly created context afterward.
    let (ids, default) = tab.cdp.contexts().await?;
    let mut allow = json!({ "behavior": "allow", "downloadPath": settings.downloads, "eventsEnabled": true });
    if let Some(context) = tab.info().await?.context {
        if ids.contains(&context) {
            allow["browserContextId"] = json!(context);
        } else if default.is_some_and(|default| default != context) {
            return Err(Error::Protocol("cannot configure downloads: unknown browser context".into()));
        }
    }
    tab.cdp.call(None, "Browser.setDownloadBehavior", allow).await?;
    Ok(())
}
