//! Bot walls: which one is standing between the agent and the page, and
//! where its widget is.

use std::sync::LazyLock;
use tokio::time::Instant;

use regex::Regex;

use serde_json::Value;

use crate::Result;
use crate::chrome::tab::Tab;
use crate::input::rect::Rect;
use crate::page::js::WALL;
use crate::page::view::{Quad, box_of};
use crate::page::world::World;
use crate::settings::Waits;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Wall {
    Cloudflare,
    /// A checkbox widget, by the host that serves it.
    Box(String),
    DataDome,
    PerimeterX,
    AwsWaf,
    /// Working out a token on its own; nothing to click, only to wait for.
    AwsWafChallenge,
    Blocked,
}

impl Wall {
    /// A word for the wall, for a tab's status.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::Box(_) => "captcha",
            Self::DataDome => "datadome",
            Self::PerimeterX => "perimeterx",
            Self::AwsWaf | Self::AwsWafChallenge => "aws-waf",
            Self::Blocked => "blocked",
        }
    }

    /// A wall worth standing at: one that may let the page through on its own,
    /// or once its box is ticked.
    pub(crate) fn passable(&self) -> bool {
        matches!(self, Self::Cloudflare | Self::Box(_) | Self::AwsWafChallenge)
    }
}

/// The frames whose checkbox gets ticked, by what their `src` matches:
/// Turnstile, reCAPTCHA's box, hCaptcha's box.
pub(crate) static CHECKBOX_FRAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"challenges\.cloudflare\.com|recaptcha/(api2|enterprise)/anchor.*size=normal|hcaptcha\.com/captcha.*frame=checkbox")
        .expect("checkbox frame pattern")
});

/// The frames a CAPTCHA draws its picture puzzle in, once the box alone did not satisfy it.
pub(crate) static CHALLENGE_FRAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"recaptcha/(api2|enterprise)/bframe|hcaptcha\.com/captcha.*frame=challenge")
        .expect("challenge frame pattern")
});

/// A frame no larger than this is an injected probe, not the widget a person sees.
const VISIBLE: (f64, f64) = (50.0, 20.0);

/// The wall standing between the agent and the page, if one is. An error means the page could not be
/// examined — a reload in progress — which is not evidence a wall is gone.
pub(crate) async fn detect(world: &mut World, tab: &Tab, waits: &Waits) -> Result<Option<Wall>> {
    let seen = world.call(tab, WALL, Vec::new()).await?;
    Ok(match seen.as_str() {
        Some("cloudflare") => Some(Wall::Cloudflare),
        Some("datadome") => Some(Wall::DataDome),
        Some("perimeterx") => Some(Wall::PerimeterX),
        Some("awswaf") => Some(Wall::AwsWaf),
        Some("awswaf-challenge") => Some(Wall::AwsWafChallenge),
        Some("blocked") => Some(Wall::Blocked),
        // A ticked widget keeps its iframe; the token is what says it is done.
        Some("cleared") => None,
        // A checkbox widget on an ordinary form hides its iframe in a closed
        // shadow root, where no selector reaches; the pierced walk that
        // finds it for the click finds it here too. A page with a place for
        // one that has not drawn it yet is given a moment.
        _ => {
            let deadline = Instant::now() + waits.widget;
            loop {
                if let Some((host, _)) = frame(tab, &CHECKBOX_FRAMES).await {
                    break Some(Wall::Box(host));
                }
                if seen != "widget" || Instant::now() >= deadline {
                    break None;
                }
                tokio::time::sleep(waits.poll).await;
            }
        }
    })
}

/// The first visible iframe whose `src` matches, wherever the page hid it:
/// its host and its box.
pub(crate) async fn frame(tab: &Tab, pattern: &Regex) -> Option<(String, Rect)> {
    let document = crate::page::dom::document(tab).await.ok()?;
    let mut frames = Vec::new();
    find_iframes(&document["root"], pattern, &mut frames);
    for (src, node) in frames {
        let Ok(Some(rect)) = box_of(tab, node, Quad::Content).await else { continue };
        if rect.width <= VISIBLE.0 || rect.height <= VISIBLE.1 {
            continue;
        }
        let host = url::Url::parse(&src).ok().and_then(|url| url.host_str().map(str::to_owned));
        return Some((host.unwrap_or(src), rect));
    }
    None
}

fn find_iframes(node: &Value, pattern: &Regex, found: &mut Vec<(String, i64)>) {
    let attributes: Vec<&str> =
        node["attributes"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if node["nodeName"].as_str().is_some_and(|name| name.eq_ignore_ascii_case("IFRAME"))
        && let Some(id) = node["backendNodeId"].as_i64()
        && let Some(src) = attributes.chunks(2).find(|pair| pair[0] == "src").and_then(|pair| pair.get(1))
        && pattern.is_match(src)
    {
        found.push(((*src).to_owned(), id));
    }
    for key in ["children", "shadowRoots"] {
        for child in node[key].as_array().into_iter().flatten() {
            find_iframes(child, pattern, found);
        }
    }
    if node["contentDocument"].is_object() {
        find_iframes(&node["contentDocument"], pattern, found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_checkbox_frame_is_found_through_a_shadow_root() {
        let document = json!({
            "nodeName": "#document", "backendNodeId": 1,
            "children": [{ "nodeName": "DIV", "backendNodeId": 2,
                "shadowRoots": [{ "nodeName": "#document-fragment", "backendNodeId": 3,
                    "children": [{ "nodeName": "IFRAME", "backendNodeId": 4,
                        "attributes": ["src", "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/turnstile", "title", "Widget"] },
                        { "nodeName": "IFRAME", "backendNodeId": 5,
                        "attributes": ["src", "https://www.google.com/recaptcha/api2/anchor?k=x&size=invisible"] }] }] }]
        });
        let mut found = Vec::new();
        find_iframes(&document, &CHECKBOX_FRAMES, &mut found);
        assert_eq!(
            found.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
            [4],
            "an invisible reCAPTCHA is not a wall"
        );
        assert!(
            CHECKBOX_FRAMES
                .is_match("https://www.google.com/recaptcha/api2/anchor?ar=1&k=x&size=normal&cb=1")
        );
    }
}
