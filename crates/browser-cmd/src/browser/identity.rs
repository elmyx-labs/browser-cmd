//! What a Chrome says it is, sampled once from the browser itself and applied
//! to every tab a headless browser opens.

use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::chrome::cdp::Cdp;
use crate::chrome::tab::{Tab, attach, create_target, watch_frames};
use crate::page::world::World;
use crate::{Error, Result, Settings};

/// Native identity is sampled once per browser on a trusted internal Chrome
/// document. about:blank has no secure-context UAData, so it is not a source
/// from which to invent GREASE, OS versions or CPU information.
#[derive(Clone)]
pub(crate) struct Identity {
    pub(crate) user_agent: String,
    pub(crate) metadata: Value,
}

/// How often the probe looks again for its document's answer: the browser's
/// own pace, not the action poll a deployment may slow.
const RECHECK: Duration = Duration::from_millis(50);

/// A cancelled probe must not leave its hidden tab behind.
struct Probe<'a>(&'a Cdp, &'a str);
impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.0.abandon_target(self.1);
    }
}

pub(crate) async fn identity(cdp: &Rc<Cdp>, settings: &Settings) -> Result<Identity> {
    let target =
        create_target(cdp, json!({"url":"chrome://version", "background":true, "hidden":true})).await?;
    let probe = Probe(cdp, &target);
    let result = tokio::time::timeout(settings.waits.launch, async {
        let tab = attach(cdp, target.clone()).await?;
        watch_frames(&tab).await?;
        let mut world = World::new();
        // Until the probe document is secure-context ready, userAgentData is
        // absent: that is an answer not yet arrived, not an answer of none.
        let mut data = loop {
            match world.call(&tab, crate::page::js::CLIENT_HINTS, Vec::new()).await {
                Ok(value) if value.is_object() => break value,
                Ok(_) | Err(Error::DocumentChanged) => tokio::time::sleep(RECHECK).await,
                Err(error) => return Err(error),
            }
        };
        if data["fullVersionList"].as_array().is_none_or(|list| list.is_empty()) {
            return Err(Error::Config(
                "native high-entropy client hints are unavailable; check global --user-agent \
                 overrides or browser policy, then restart Chrome"
                    .into(),
            ));
        }
        if let Some(full) = data.as_object_mut().and_then(|map| map.remove("uaFullVersion")) {
            data["fullVersion"] = full;
        }
        Ok(Identity { user_agent: cdp.user_agent().await?, metadata: data })
    })
    .await
    .unwrap_or(Err(Error::Timeout("native browser identity")));
    let closed = tokio::time::timeout(settings.waits.close, cdp.close_target(&target))
        .await
        .unwrap_or(Err(Error::Timeout("identity probe cleanup")));
    if closed.is_ok() {
        std::mem::forget(probe);
    }
    result.and_then(|identity| closed.map(|()| identity))
}

pub(crate) async fn apply_identity(tab: &Tab, identity: &Identity) -> Result<()> {
    // Headless is the only reason an identity is applied: its user agent says
    // so, and nothing else about the browser needs to change.
    let user_agent = identity.user_agent.replace("HeadlessChrome", "Chrome");
    tab.call(
        "Network.setUserAgentOverride",
        json!({"userAgent":user_agent,"userAgentMetadata":identity.metadata}),
    )
    .await?;
    warn_if_software_rendered(tab, &mut World::new()).await;
    Ok(())
}

/// A headless Chrome with no GPU falls back to a software rasterizer, and its
/// WebGL renderer names it — the one headless tell that no string can hide,
/// because software rendering has numeric quirks a spoofed name would still
/// contradict. It is honest, not silent: the operator is told once, and told
/// the fix, which is a real GPU or a headed browser under a virtual display
/// (`xvfb-run` on Linux) rather than headless.
async fn warn_if_software_rendered(tab: &Tab, world: &mut World) {
    use std::sync::atomic::{AtomicBool, Ordering};
    // The GPU is the machine's: one look per process answers for every tab.
    static PROBED: AtomicBool = AtomicBool::new(false);
    if PROBED.swap(true, Ordering::Relaxed) {
        return;
    }
    let renderer = crate::page::canvas::webgl_renderer(world, tab)
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let software = ["swiftshader", "llvmpipe", "software", "microsoft basic"]
        .iter()
        .any(|name| renderer.to_ascii_lowercase().contains(name));
    if software {
        eprintln!(
            "browser-cmd: WebGL is software-rendered ({renderer}); a bot wall can tell this from a \
             real GPU. For full stealth, run under a virtual display with a GPU (e.g. `xvfb-run -a \
             browser-cmd ...`) so the browser is headed, or give the host a GPU."
        );
    }
}
