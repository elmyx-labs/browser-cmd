//! What stands over the page without being a bot wall: a consent dialog, a
//! sign-up modal, a paywall. Named, never answered: what a person accepts about
//! their own privacy is not this tool's decision.

use crate::chrome::tab::Tab;
use crate::page::js::OVERLAY;
use crate::page::world::World;

/// What is over the page: what it says, the ways out it offers, and whether it is a dialog.
pub(crate) struct Overlay {
    pub(crate) what: String,
    pub(crate) choices: Vec<String>,
    pub(crate) modal: bool,
}

/// What is over the page, if anything is.
pub(crate) async fn over_the_page(world: &mut World, tab: &Tab) -> Option<Overlay> {
    let found = world.call(tab, OVERLAY, Vec::new()).await.ok()?;
    let what = found.get("what")?.as_str()?.trim().to_owned();
    let choices: Vec<String> =
        found["choices"].as_array()?.iter().filter_map(|choice| choice.as_str()).map(str::to_owned).collect();
    if what.is_empty() && choices.is_empty() {
        return None;
    }
    Some(Overlay { what, choices, modal: found["modal"] == true })
}
