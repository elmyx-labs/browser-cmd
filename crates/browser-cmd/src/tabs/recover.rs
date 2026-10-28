//! Reloading tabs whose renderer crashed, reopening kept tabs whose page went
//! away, and taking a session's tabs back after a restart of the daemon.

use std::collections::HashSet;
use std::rc::Rc;

use crate::browser::host::Shared;
use crate::page::navigation::{Landing, navigate, reload};
use crate::tabs::context::BrowserContext;
use crate::tabs::ledger::{Class, Hold};
use crate::tabs::status::TabStatus;
use crate::tabs::{Holder, attach, fall_back, fresh, page_ids, target_id};
use crate::{Error, Result};

/// After a daemon restart, before this session's first command: its tabs
/// still open taken back under their handles, owned or borrowed as they were,
/// with its own context and the tab it was on; a kept tab no longer open is
/// gone until `tab restore`, said once.
pub(crate) async fn resume(holder: &mut impl Holder) -> Result<Option<String>> {
    let Some(earlier) = holder.book().earlier.clone() else { return Ok(None) };
    let host = holder.host();
    let shared = host.shared().await?;
    let open = page_ids(&host).await?;
    let mut owned = Vec::new();
    for id in earlier.context.iter().chain(&earlier.retired) {
        let exists = shared.cdp.context_exists(id).await?;
        owned.push(exists.then(|| BrowserContext::new(shared.cdp.clone(), id.clone())));
    }
    let context = if earlier.context.is_some() { owned.remove(0) } else { None };
    let book = holder.book();
    let (ledger, name) = (book.ledger.clone(), book.name.clone());
    let taken = |target: &String| {
        open.contains(target)
            && matches!(ledger.borrow().class(target, &name), Class::Unowned | Class::Own | Class::Borrowed)
    };
    let live: HashSet<String> =
        book.registry.entries.iter().map(|entry| &entry.target).filter(|t| taken(t)).cloned().collect();
    (book.context, book.retired) = (context, owned.into_iter().flatten().collect());
    book.registry.resume(&live, book.context.is_some());
    for entry in book.registry.entries.iter().filter(|entry| live.contains(&entry.target)) {
        let hold = if entry.borrowed { Hold::Borrowed(name.clone()) } else { Hold::Owned(name.clone()) };
        ledger.borrow_mut().hold(&entry.target, hold);
    }
    book.previous = earlier.previous.into_iter().filter(|target| live.contains(target)).collect();
    book.earlier = None;
    if let Some(active) = earlier.active.filter(|target| live.contains(target)) {
        attach(holder, &active).await?;
    }
    let gone: Vec<String> = holder
        .book()
        .registry
        .entries
        .iter()
        .filter(|entry| entry.gone())
        .map(|entry| entry.name())
        .collect();
    Ok(match gone.len() {
        0 => None,
        1 => Some(format!("Kept tab {} did not outlast an earlier run; `tab restore` reopens it.", gone[0])),
        _ => Some(format!(
            "Kept tabs {} did not outlast an earlier run; `tab restore` reopens them.",
            gone.join(", ")
        )),
    })
}

/// Reopen, each at the address it was kept at, the kept tabs whose page is
/// gone — the browser replaced, a tab crashed or closed — in the context each
/// lived in. The active tab stays unless it went too. Says which, in one line.
pub(crate) async fn restore(holder: &mut impl Holder) -> Result<Option<String>> {
    let book = holder.book();
    if book.earlier.is_some() || book.registry.kept().all(|entry| entry.gone()) {
        return Ok(None);
    }
    let host = holder.host();
    let shared = host.shared().await?;
    let open = page_ids(&host).await?;
    let book = holder.book();
    if book.registry.kept().all(|entry| entry.gone() || open.contains(&entry.target)) {
        return Ok(None);
    }
    let replaced = book.active.as_ref().is_none_or(|tab| !Rc::ptr_eq(&tab.cdp, &shared.cdp));
    let (mut said, names) = reopen(holder, &shared, &open).await?;
    let (tabs, cause) = match (names.len(), replaced) {
        (0, _) => return Ok(Some(said.trim_end().to_owned()).filter(|said| !said.is_empty())),
        (1, true) => ("tab", "the browser was replaced"),
        (_, true) => ("tabs", "the browser was replaced"),
        (1, false) => ("tab", "its page closed or crashed"),
        (_, false) => ("tabs", "their pages closed or crashed"),
    };
    said.push_str(&format!("Restored kept {tabs} {} after {cause}.", names.join(", ")));
    Ok(Some(said))
}

/// `tab restore`: reopen the kept tabs an earlier run left gone, the one named or all.
pub(crate) async fn bring_back(holder: &mut impl Holder, word: Option<&str>) -> Result<String> {
    let host = holder.host();
    let shared = host.shared().await?;
    let open = page_ids(&host).await?;
    holder.book().registry.revive(word)?;
    let (said, names) = reopen(holder, &shared, &open).await?;
    match names.is_empty() {
        true => Err(Error::Tool(format!(
            "{said}nothing was restored: the browser context those tabs lived in is gone"
        ))),
        false => Ok(format!(
            "{said}Restored kept {} {}.",
            if names.len() == 1 { "tab" } else { "tabs" },
            names.join(", ")
        )),
    }
}

/// Reopen the kept tabs to reopen, each in the kind of context it lived in;
/// when the active tab went, go to the next: a lost context of this
/// session's own said, and the handles reopened.
async fn reopen(
    holder: &mut impl Holder,
    shared: &Rc<Shared>,
    open: &HashSet<String>,
) -> Result<(String, Vec<String>)> {
    let host = holder.host();
    let book = holder.book();
    let active = book.active.as_ref().map(|tab| tab.target.clone());
    let survives = active.as_ref().is_some_and(|target| open.contains(target));
    // A context lost with the browser is replaced here, and said; the tabs that lived in it went with it.
    let (said, context) = match book.reconcile(shared).await {
        Err(Error::Tool(why)) => (format!("{why}.\n"), false),
        result => result.map(|()| (String::new(), book.context.is_some()))?,
    };
    let gone = book.registry.reopenable(open, context);
    let mut names = Vec::new();
    for (target, url) in gone {
        let book = holder.book();
        let isolated = book.registry.of(&target).is_some_and(|entry| entry.isolated);
        let tab = fresh(shared, host.settings(), book, isolated).await?;
        let navigated = navigate(&tab, &url, host.settings().waits.navigate).await;
        let book = holder.book();
        book.retarget(&target, &tab.target);
        let entry = book.own(&tab.target);
        entry.status = match navigated {
            Ok(Landing::Committed) => TabStatus::Restored,
            Ok(Landing::Failed(why)) => TabStatus::Failed(why),
            _ => TabStatus::NotLoaded,
        };
        names.push(entry.name());
    }
    if !survives && !names.is_empty() {
        fall_back(holder).await?;
    }
    Ok((said, names))
}

/// Reload in place, under the same handle, this session's tabs whose renderer
/// crashed since the last command, each said once.
pub(crate) async fn crashed(holder: &mut impl Holder) -> Vec<String> {
    let Some(shared) = holder.host().running().await else { return Vec::new() };
    let crashed: Vec<String> = {
        let targets = &shared.cdp.events().targets;
        let pages = targets.pages().iter().filter(|page| page["crashed"] == true);
        pages.map(|page| target_id(page).to_owned()).collect()
    };
    let mut notes = Vec::new();
    for target in crashed {
        let Some(name) = holder.book().registry.of(&target).map(|entry| entry.name()) else { continue };
        shared.cdp.events().targets.recovered(&target);
        let tab = match holder.book().live(&shared).filter(|tab| tab.target == target) {
            Some(tab) => Ok(tab),
            None => crate::chrome::tab::attach(&shared.cdp, target.clone()).await,
        };
        let settings = holder.host().settings().clone();
        let reload = async {
            let tab = tab?;
            reload(&tab).await?;
            crate::page::settle::settle(&mut crate::page::world::World::new(), &tab, &settings.waits).await;
            Ok::<_, Error>(tab)
        };
        notes.push(match tokio::time::timeout(settings.waits.navigate, reload).await {
            Ok(Ok(tab)) => {
                tab.cdp.events().reloaded(&tab.session, &target);
                format!("{name} crashed and was reloaded.")
            }
            Ok(Err(error)) => format!("{name} crashed and could not be reloaded: {error}."),
            Err(_) => format!("{name} crashed and did not answer a reload."),
        });
        if holder.book().active.as_ref().is_some_and(|tab| tab.target == target) {
            holder.switched(Some(target.clone()), Some(target));
        }
    }
    notes
}
