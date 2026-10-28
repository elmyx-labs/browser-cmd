//! Where a page comes to rest once it moved: settled, then past a wall or
//! still behind one, and its status from the document it holds.

use serde_json::Value;

use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::page::settle::{Settled, settle};
use crate::page::walls::Wall;
use crate::page::world::World;
use crate::session::{notes, wall};
use crate::settings::Waits;
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
    /// refusal when that is what the document is.
    pub(crate) fn verdict(&self, address: &str, document: &Value) -> (TabStatus, Option<String>) {
        (crate::tabs::status::status(address, document, self.wall.as_ref()), notes::refused(document))
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
