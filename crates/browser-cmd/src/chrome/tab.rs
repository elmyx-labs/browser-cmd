//! A tab: opening and attaching to a page target, what the browser says of
//! it, and the frames it reports.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::chrome::cdp::Cdp;
use crate::{Error, Result};

/// One attached tab: the target and the session that speaks for it.
#[derive(Clone)]
pub(crate) struct Tab(Rc<Attachment>);

pub(crate) struct Attachment {
    pub(crate) cdp: Rc<Cdp>,
    pub(crate) target: String,
    pub(crate) session: String,
}

impl std::ops::Deref for Tab {
    type Target = Attachment;
    fn deref(&self) -> &Attachment {
        &self.0
    }
}

impl Drop for Attachment {
    /// Last-owner cleanup also covers failed preparation and abandoned handler
    /// futures; the browser's detach event reclaims session-local bookkeeping.
    fn drop(&mut self) {
        self.cdp.detach(&self.session, &self.target);
    }
}

impl Tab {
    pub(crate) fn new(cdp: Rc<Cdp>, target: String, session: String) -> Self {
        Self(Rc::new(Attachment { cdp, target, session }))
    }

    /// A tab no browser attached, for tests.
    #[cfg(test)]
    pub(crate) fn fixture(cdp: Rc<Cdp>, target: String, session: String) -> Self {
        cdp.events().attachments.insert(session.clone());
        Self::new(cdp, target, session)
    }

    pub(crate) fn attached(&self) -> bool {
        self.cdp.alive() && self.cdp.events().attachments.contains(&self.session)
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.cdp.call(Some(&self.session), method, params).await
    }

    /// Fire-and-forget dispatch for the tab; see [`Cdp::send`].
    pub(crate) fn send(&self, method: &str, params: Value) -> Result<()> {
        self.cdp.send(Some(&self.session), method, params)
    }

    /// Where the tab is, from the browser's own target list rather than from
    /// anything the page could have a say in.
    pub(crate) async fn url(&self) -> Result<String> {
        Ok(self.info().await?.url)
    }

    /// What the browser says of this tab: where it is, its title, its browser context.
    pub(crate) async fn info(&self) -> Result<TargetInfo> {
        let info = self.cdp.call(None, "Target.getTargetInfo", json!({ "targetId": self.target })).await?;
        let info = &info["targetInfo"];
        if info["targetId"] != self.target.as_str() {
            return Err(Error::Protocol("the browser described another target than this tab".into()));
        }
        let context = match info.get("browserContextId") {
            None => None,
            Some(id) => Some(
                id.as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| Error::Protocol("the tab has an invalid browser context".into()))?
                    .to_owned(),
            ),
        };
        Ok(TargetInfo {
            url: info["url"].as_str().unwrap_or("about:blank").to_owned(),
            title: info["title"].as_str().unwrap_or_default().to_owned(),
            context,
        })
    }

    /// Bring the tab to the front.
    pub(crate) async fn activate(&self) -> Result<()> {
        self.cdp.call(None, "Target.activateTarget", json!({ "targetId": self.target })).await.map(drop)
    }
}

pub(crate) struct TargetInfo {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) context: Option<String>,
}

/// Open a tab, in a context of its own when asked, and attach to it; never an
/// existing blank page. `apart` opens it in a window of its own, behind the
/// windows a person is using.
pub(crate) async fn new_tab(cdp: &Rc<Cdp>, context: Option<&str>, apart: bool) -> Result<Tab> {
    let mut params = json!({ "url": "about:blank" });
    if apart {
        (params["newWindow"], params["background"]) = (true.into(), true.into());
    }
    if let Some(context) = context {
        params["browserContextId"] = Value::String(context.to_owned());
    }
    let target = create_target(cdp, params).await?;
    attach(cdp, target.clone()).await.inspect_err(|_| cdp.abandon_target(&target))
}

/// A page target made as `Target.createTarget` is told, not attached to; its identity.
pub(crate) async fn create_target(cdp: &Cdp, params: Value) -> Result<String> {
    let created = cdp.call(None, "Target.createTarget", params).await?;
    let target = created["targetId"].as_str().filter(|id| !id.is_empty()).map(str::to_owned);
    target.ok_or_else(|| Error::Tool("the browser opened no tab".into()))
}

pub(crate) async fn attach(cdp: &Rc<Cdp>, target: String) -> Result<Tab> {
    let attached =
        cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or_else(|| Error::Tool("the tab gave no session".into()))?
        .to_owned();
    Ok(Tab::new(cdp.clone(), target, session))
}

/// Enable the Page domain — for the dialog events, and with no script-visible
/// side — and seed frame ownership from the tree that already exists: an open
/// document need not emit `frameNavigated` after the enable, and ownership has
/// to be known before an action can start a download.
pub(crate) async fn watch_frames(tab: &Tab) -> Result<()> {
    tab.call("Page.enable", json!({})).await?;
    let frames = tab.call("Page.getFrameTree", json!({})).await?;
    tab.cdp.events().register_frames(&tab.session, &frames["frameTree"]);
    Ok(())
}
