//! Closing tabs: one by name, the ones not kept, one that hung, all of a
//! session that ends. A borrowed tab is never closed: it is given back, left open.

use std::collections::HashSet;

use serde_json::Value;

use crate::Result;
use crate::chrome::cdp::Cdp;
use crate::chrome::events::Note;
use crate::tabs::book::Book;
use crate::tabs::{Holder, fall_back, leave, switch, target_id};

/// How long a tab gets to answer `1` before it counts as hung.
const HUNG_PROBE: std::time::Duration = std::time::Duration::from_secs(3);

impl Book {
    /// Close a tab of this session's own, or give back a borrowed one.
    pub(crate) async fn let_go(&self, cdp: &Cdp, target: &str) -> Result<()> {
        match self.registry.of(target).is_some_and(|entry| entry.borrowed) {
            true => self.ledger.borrow_mut().release(target),
            false => cdp.close_target(target).await?,
        }
        Ok(())
    }
}

/// Close a tab; when it was the active one, go to the next, and say where that is.
pub(crate) async fn close(holder: &mut impl Holder, target: &str) -> Result<Option<String>> {
    let shared = holder.host().shared().await?;
    holder.book().let_go(&shared.cdp, target).await?;
    holder.book().registry.forget(target);
    leave(holder, target).await
}

/// Close this session's tabs that are not kept; never another session's. The
/// ones closed, the ones kept, and the one switched to when the active one closed.
pub(crate) async fn tidy(
    holder: &mut impl Holder,
    pages: &[Value],
) -> Result<(Vec<String>, Vec<String>, Option<String>)> {
    let kept: Vec<String> = holder.book().registry.kept().map(|entry| entry.name()).collect();
    if kept.is_empty() {
        return Err(crate::Error::Tool(
            "no tab is kept, so none was closed; `tab keep` marks the ones to stay".into(),
        ));
    }
    let shared = holder.host().shared().await?;
    let active = holder.book().active.as_ref().map(|tab| tab.target.clone());
    let (mut closed, mut left) = (Vec::new(), false);
    for entry in holder.book().registry.entries.clone() {
        if entry.kept.is_none() && !entry.borrowed && pages.iter().any(|page| target_id(page) == entry.target)
        {
            shared.cdp.close_target(&entry.target).await?;
            holder.book().registry.forget(&entry.target);
            closed.push(entry.name());
            left |= active.as_ref() == Some(&entry.target);
        }
    }
    let switched = match left {
        true => {
            fall_back(holder).await?.and_then(|(tab, _)| Some(holder.book().registry.of(&tab.target)?.name()))
        }
        false => None,
    };
    Ok((closed, kept, switched))
}

/// What ending a session did, by handle.
#[derive(Default)]
pub(crate) struct Ended {
    pub(crate) closed: Vec<String>,
    pub(crate) given: Vec<String>,
    pub(crate) kept: Vec<String>,
}

/// End this session's tabs: its own not kept closed, borrowed ones given
/// back, kept ones left open, and none active.
pub(crate) async fn end(holder: &mut impl Holder) -> Result<Ended> {
    let mut ended = Ended::default();
    if let Some(shared) = holder.host().running().await {
        let open: HashSet<String> =
            shared.cdp.events().targets.pages().iter().map(|page| target_id(page).to_owned()).collect();
        let active = holder.book().active.as_ref().map(|tab| tab.target.clone());
        for entry in holder.book().registry.entries.clone() {
            if !open.contains(&entry.target) && active.as_ref() != Some(&entry.target) {
                continue;
            }
            if entry.kept.is_some() {
                ended.kept.push(entry.name());
                continue;
            }
            holder.book().let_go(&shared.cdp, &entry.target).await?;
            holder.book().registry.forget(&entry.target);
            if entry.borrowed { &mut ended.given } else { &mut ended.closed }.push(entry.name());
        }
    }
    switch(holder, None);
    Ok(ended)
}

/// Close a tab that no longer answers even `1`, so the next command does not
/// wait out every timeout, and go to the next: what it had to say, and where
/// that is. A live browser is what tells a hung tab from a hung browser, which
/// the host restarts.
pub(crate) async fn replace_hung(holder: &mut impl Holder) -> Option<(Vec<Note>, Option<String>)> {
    let tab = holder.book().active.clone()?;
    if crate::page::world::answers(&tab, HUNG_PROBE).await || !tab.cdp.alive() {
        return None;
    }
    let notes = tab.cdp.events().take_notes(&tab.session, &tab.target);
    let _ = holder.book().let_go(&tab.cdp, &tab.target).await;
    if holder.book().registry.of(&tab.target).is_some_and(|entry| entry.kept.is_none()) {
        holder.book().registry.forget(&tab.target);
    }
    let next = fall_back(holder).await.ok().flatten();
    Some((notes, next.map(|(_, url)| url)))
}
