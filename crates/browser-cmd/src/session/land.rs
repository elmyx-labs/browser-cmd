//! Where a page comes to rest once it moved: settled, then past a wall or
//! still behind one, and its status from the document it holds.

use serde_json::Value;

use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::page::settle::{Settled, settle};
use crate::page::walls::Wall;
use crate::page::world::World;
use crate::session::{notes, wall};
use crate::settings::{Settings, Waits};
use crate::tabs::status::TabStatus;

pub(crate) struct Landed {
    pub(crate) settled: Settled,
    /// What became of a wall in front of the page.
    pub(crate) said: Option<String>,
    /// The wall still standing.
    wall: Option<Wall>,
}

impl Landed {
    /// The tab's status from the document it landed on, and the server's
    /// refusal when that is what the document is. A page that turned our own
    /// headless browser away is told so: it is a fact of the environment the
    /// agent cannot see from the page.
    pub(crate) fn verdict(
        &self,
        address: &str,
        document: &Value,
        settings: &Settings,
    ) -> (TabStatus, Option<String>) {
        let status = crate::tabs::status::status(address, document, self.wall.as_ref());
        let turned_away = matches!(status, TabStatus::Wall(_) | TabStatus::Blocked | TabStatus::Http(403));
        let headless = (turned_away && settings.headless && settings.attach.is_none()).then(notes::headless);
        let said: Vec<String> = notes::refused(document).into_iter().chain(headless).collect();
        (status, (!said.is_empty()).then(|| said.join(" ")))
    }
}

/// Settle the page, then meet whatever wall stands in front of it.
pub(crate) async fn land(world: &mut World, tab: &Tab, waits: &Waits, hand: &mut Hand) -> Landed {
    let settled = settle(world, tab, waits).await;
    let Some(stand) = wall::face(world, tab, waits, hand).await else {
        return Landed { settled, said: None, wall: None };
    };
    Landed {
        settled: stand.stood.as_ref().map_or(settled, |(_, _, after)| *after),
        said: Some(notes::wall(&stand)),
        wall: stand.standing().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_page_that_turned_our_headless_browser_away_is_told_it_is_headless() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::new(temp.path(), temp.path());
        let landed = |wall| Landed { settled: Settled { capped: None }, said: None, wall };
        let note = |landed: &Landed, status: u64, settings: &Settings| {
            landed.verdict("https://a.test/", &json!({ "status": status }), settings).1.unwrap_or_default()
        };
        assert!(note(&landed(Some(Wall::Cloudflare)), 200, &settings).contains("headless"));
        assert!(note(&landed(None), 403, &settings).contains("headless"));
        assert!(
            !note(&landed(None), 404, &settings).contains("headless"),
            "a missing page is no refusal of us"
        );
        assert_eq!(note(&landed(None), 200, &settings), "");
        settings.headless = false;
        assert!(!note(&landed(Some(Wall::Blocked)), 403, &settings).contains("headless"));
        settings.headless = true;
        settings.attach = Some("http://127.0.0.1:9222".into());
        assert!(!note(&landed(None), 403, &settings).contains("headless"), "somebody's browser is as it is");
    }
}
