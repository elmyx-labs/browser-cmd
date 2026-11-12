//! What every action ends with: settling, walls, overlays, refusals, PDFs, what
//! the page draws, and the tab's state with whatever the page newly says.

use crate::chrome::tab::Tab;
use crate::page::overlay::{Overlay, over_the_page};
use crate::session::land::{Landed, land};
use crate::session::pdf::saved_pdf;
use crate::session::state::State;
use crate::session::visual::visual;
use crate::session::{Session, notes};
use crate::text::fence::bounded_state;
use crate::{Error, Result};

/// How many times a state is captured again when the page re-rendered under the capture.
const RECAPTURES: usize = 2;

/// Land the page and gather what happened while nobody asked, and what stands over it.
async fn settle_notes(session: &mut Session, tab: &Tab) -> (Landed, Vec<String>, Option<Overlay>) {
    let host = session.host.clone();
    let landed = land(&mut session.document.world, tab, &host.settings().waits, &mut session.hand).await;
    let mut notes: Vec<String> =
        tab.cdp.events().take_notes(&tab.session, &tab.target).iter().map(notes::event).collect();
    notes.extend(landed.said.clone());
    let over = over_the_page(&mut session.document.world, tab).await;
    (landed, notes, over)
}

/// The tab's state, captured again once the page rests when a re-render raced
/// the capture: the action is done and is never what is repeated.
async fn captured(session: &mut Session, tab: &Tab) -> Result<State> {
    let mut left = RECAPTURES;
    loop {
        match session.document.state(tab, false, false).await {
            Err(Error::DocumentChanged) if left > 0 => {
                left -= 1;
                session.settle(tab).await;
            }
            state => return state,
        }
    }
}

pub(crate) async fn after(session: &mut Session, tab: &Tab, action: &str) -> Result<String> {
    // A tab the action opened is where a person would now be looking, and one
    // it closed is gone: either way the state is of the tab followed to.
    let (tab, followed) = crate::tabs::follow(session, tab).await?;
    let tab = &tab;
    let (landed, mut notes, over) = settle_notes(session, tab).await;
    notes.splice(0..0, followed);
    if std::mem::take(&mut session.book.lost) {
        notes.insert(
            0,
            "The browser had stopped answering or exited and was started again: the tabs open before \
             are gone and this is a new one."
                .into(),
        );
    }
    let state = captured(session, tab).await?;
    let empty = state.empty;
    // After the state, so each way out is named by the ref that state gave it.
    notes.extend(over.map(|over| notes::overlay(&over, |name| session.document.refs.only_named(name))));
    let (status, refused) = landed.verdict(&state.address, &state.document);
    notes.extend(session.book.registry.seen(&tab.target, &state.address, &state.title, status));
    notes.extend(refused);
    notes.extend(saved_pdf(session, tab, &state.document).await);
    // On a document already shown, what the action changed is the news; the
    // controls that stayed put were in the last state and stay addressable.
    let compact = session.document.refs.compact(session.document.told.contains("delta"));
    if compact.is_some() {
        session.document.told.insert("delta");
    }
    let mut state = session.document.shown(state, compact);
    // The message an action produces usually has no control of its own, so
    // the tree never shows it. It goes inside the state because it is the
    // page's own words and the fence is what makes them quotable.
    if let Some(said) = session.document.new_text(tab, session.host.settings().limits.new_text).await {
        match state.find("\n\n") {
            Some(end_of_header) => state.insert_str(end_of_header + 2, &format!("{said}\n\n")),
            None => state.push_str(&format!("\n\n{said}")),
        }
    }
    notes.extend(visual(session, tab, empty).await);
    let capped = notes::capped(landed.settled).map_or_else(String::new, |warning| format!(" {warning}"));
    let notes = notes.iter().map(|note| format!("\n{note}")).collect::<String>();
    Ok(format!("{action}.{capped}{notes}\n{}", bounded_state(state, session.settings().limits.state)))
}
