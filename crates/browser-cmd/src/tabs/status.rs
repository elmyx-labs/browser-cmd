//! A tab's status in a word or two: loaded, an error page, a refusal, a wall, empty.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
use crate::Settings;
use crate::chrome::tab::Tab;
use crate::page::walls::Wall;
use crate::page::world::World;

/// What a tab last showed, or what became of it; kept in its file as `tab list` words it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub(crate) enum TabStatus {
    #[default]
    Unchecked,
    Ok,
    Blank,
    /// The browser's own error page.
    Error,
    /// A navigation the browser gave up on, and its reason.
    Failed(String),
    Http(u64),
    Blocked,
    Wall(String),
    Restored,
    NotLoaded,
    Unreadable(String),
    Closed,
    /// Kept, not open since an earlier run of the daemon: reopened only by `tab restore`.
    Earlier,
    /// Kept, and its own context is gone: never reopened in another.
    Lost,
    /// Read while its document was still `loading` or `interactive`.
    Loading(Box<TabStatus>, String),
}

impl TabStatus {
    /// Kept, and not to be reopened by itself.
    pub(crate) fn gone(&self) -> bool {
        matches!(self, Self::Earlier | Self::Lost)
    }
}

impl fmt::Display for TabStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unchecked => write!(f, "unchecked"),
            Self::Ok => write!(f, "ok"),
            Self::Blank => write!(f, "blank"),
            Self::Error => write!(f, "error"),
            Self::Failed(why) => write!(f, "error {why}"),
            Self::Http(code) => write!(f, "http {code}"),
            Self::Blocked => write!(f, "blocked"),
            Self::Wall(kind) => write!(f, "wall {kind}"),
            Self::Restored => write!(f, "restored"),
            Self::NotLoaded => write!(f, "restored, not loaded"),
            Self::Unreadable(why) => write!(f, "unreadable: {why}"),
            Self::Closed => write!(f, "closed"),
            Self::Earlier => write!(f, "gone (earlier run)"),
            Self::Lost => write!(f, "gone (context lost)"),
            Self::Loading(status, ready) => write!(f, "{status}, {ready}"),
        }
    }
}

impl From<String> for TabStatus {
    /// A status as a file holds it; one no longer known is unchecked.
    fn from(word: String) -> Self {
        let fixed = [
            Self::Ok,
            Self::Blank,
            Self::Error,
            Self::Blocked,
            Self::Restored,
            Self::NotLoaded,
            Self::Closed,
            Self::Earlier,
            Self::Lost,
        ];
        if let Some(found) = fixed.into_iter().find(|status| status.to_string() == word) {
            return found;
        }
        let rest = |prefix: &str| word.strip_prefix(prefix).map(str::to_owned);
        if let Some(why) = rest("unreadable: ") {
            return Self::Unreadable(why);
        }
        if let Some((status, ready)) = word.rsplit_once(", ") {
            return Self::Loading(Box::new(status.to_owned().into()), ready.into());
        }
        match (rest("error "), rest("http ").and_then(|code| code.parse().ok()), rest("wall ")) {
            (Some(why), ..) => Self::Failed(why),
            (_, Some(code), _) => Self::Http(code),
            (.., Some(kind)) => Self::Wall(kind),
            _ => Self::Unchecked,
        }
    }
}

impl From<TabStatus> for String {
    fn from(status: TabStatus) -> Self {
        status.to_string()
    }
}

/// A tab's status from what an action already read of it: an error page, the
/// server's refusal, a wall, nothing on it, or ok.
pub(crate) fn status(url: &str, document: &Value, wall: Option<&Wall>) -> TabStatus {
    let code = document["status"].as_u64().unwrap_or_default();
    match wall {
        _ if document["error"] == true => TabStatus::Error,
        _ if code >= 400 => TabStatus::Http(code),
        Some(Wall::Blocked) => TabStatus::Blocked,
        Some(wall) => TabStatus::Wall(wall.kind().into()),
        None if url == "about:blank" || document["text"] == false => TabStatus::Blank,
        None => TabStatus::Ok,
    }
}

/// A tab's status read live, with how far it has loaded when not yet complete.
pub(crate) async fn read(tab: &Tab, world: &mut World, settings: &Settings) -> Result<TabStatus> {
    let document = world.call(tab, crate::page::js::DOCUMENT, Vec::new()).await?;
    let wall = crate::page::walls::detect(world, tab, &settings.waits).await.ok().flatten();
    let now = status(&tab.url().await?, &document, wall.as_ref());
    Ok(match document["ready"].as_str().filter(|ready| *ready != "complete") {
        Some(ready) => TabStatus::Loading(Box::new(now), ready.into()),
        None => now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::registry::Registry;
    use serde_json::json;

    #[test]
    fn status_names_error_pages_refusals_walls_and_blank_pages() {
        let page = json!({"status": 200, "text": true});
        let said = |url: &str, document: &Value, wall: Option<&Wall>| status(url, document, wall).to_string();
        assert_eq!(said("https://a.test/", &page, None), "ok");
        assert_eq!(
            said("https://a.test/", &json!({"status": 0, "text": true, "error": true}), None),
            "error"
        );
        assert_eq!(said("https://a.test/", &json!({"status": 404, "text": true}), None), "http 404");
        assert_eq!(said("https://a.test/", &page, Some(&Wall::Blocked)), "blocked");
        assert_eq!(said("https://a.test/", &page, Some(&Wall::Cloudflare)), "wall cloudflare");
        assert_eq!(said("about:blank", &page, None), "blank");
        assert_eq!(said("https://a.test/", &json!({"status": 200, "text": false}), None), "blank");
        let mut registry = Registry::default();
        let failed = TabStatus::Failed("net::ERR_NAME_NOT_RESOLVED".into());
        registry.adopt("E").status = failed.clone();
        registry.seen("E", "https://e.test/", "", TabStatus::Error);
        assert_eq!(registry.of("E").unwrap().status, failed);
    }

    #[test]
    fn a_status_reads_back_from_its_words() {
        for word in [
            "unchecked",
            "ok",
            "error",
            "error net::ERR_TIMED_OUT",
            "http 404",
            "wall aws-waf",
            "restored, not loaded",
            "unreadable: tool error: the page re-rendered, twice",
            "gone (earlier run)",
            "http 503, interactive",
        ] {
            assert_eq!(TabStatus::from(word.to_owned()).to_string(), word);
        }
        assert_eq!(TabStatus::from("something else".to_owned()), TabStatus::Unchecked);
    }
}
