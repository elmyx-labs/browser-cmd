//! Which tab a session is on and the tabs it holds, following a tab an action
//! opens or closes, taking the ones it comes to hold, reloading crashed ones,
//! reopening kept ones that went away, taking back its tabs after a daemon
//! restart, and the browser context it owns after a clean retry.

pub(crate) mod book;
pub(crate) mod close;
pub(crate) mod context;
pub(crate) mod cookies;
pub(crate) mod ledger;
pub(crate) mod recover;
pub(crate) mod registry;
pub(crate) mod status;
pub(crate) mod store;

use std::collections::HashSet;
use std::rc::Rc;

use serde_json::Value;

use crate::browser::host::{Host, Shared};
use crate::browser::prepare::prepare;
use crate::chrome::tab::{Tab, new_tab};
use crate::tabs::book::Book;
use crate::tabs::ledger::Hold;
use crate::{Error, Result, Settings};

/// What holds a book: the session, which keeps what it knew of each tab's document.
pub(crate) trait Holder {
    fn book(&mut self) -> &mut Book;
    fn host(&self) -> Rc<Host>;
    /// The active tab went from one target to another, or to the same one again.
    fn switched(&mut self, left: Option<String>, next: Option<String>);
}

/// A tab's identity, from a list [`pages`] has already refused
/// to return if any entry lacked one.
pub(crate) fn target_id(tab: &Value) -> &str {
    tab["targetId"].as_str().unwrap_or_default()
}

/// The browser's pages as it lists them now, which the ledger of pages takes as its own.
pub(crate) async fn pages(host: &Host) -> Result<Vec<Value>> {
    let shared = host.shared().await?;
    let pages: Vec<Value> =
        shared.cdp.targets().await?.into_iter().filter(|target| target["type"] == "page").collect();
    let pages = {
        let targets = &mut shared.cdp.events().targets;
        targets.seed(&pages);
        targets.pages().to_vec()
    };
    ledger::of(host).borrow_mut().sweep(&pages);
    Ok(pages)
}

/// The one open page whose id starts so: four hex digits or more, not all
/// decimal, which name a handle or a place in a list instead.
pub(crate) fn unique(pages: &[Value], id: &str) -> Result<String> {
    let hex = id.len() >= 4 && id.chars().all(|c| c.is_ascii_hexdigit());
    if !hex || id.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::Tool(format!("no open tab is {id}; run tab list")));
    }
    let found: Vec<&str> = pages.iter().map(target_id).filter(|target| target.starts_with(id)).collect();
    match found[..] {
        [one] => Ok(one.to_owned()),
        [] => Err(Error::Tool(format!("no open tab is {id}; run tab list"))),
        _ => Err(Error::Tool("several open tabs have ids starting so; give more of the id".into())),
    }
}

pub(crate) async fn page_ids(host: &Host) -> Result<HashSet<String>> {
    Ok(pages(host).await?.iter().map(|page| target_id(page).to_owned()).collect())
}

/// The browser's pages, with this session's tabs that closed forgotten unless kept.
pub(crate) async fn survey(holder: &mut impl Holder) -> Result<Vec<Value>> {
    let pages = pages(&holder.host()).await?;
    holder.book().registry.prune(&pages.iter().map(|page| target_id(page).to_owned()).collect());
    Ok(pages)
}

/// A tab for a session, in its own context when it has one and `own_context`
/// asks for it: in the default context the blank page Chrome opened at
/// launch, while no session has claimed it, else a new one.
pub(crate) async fn fresh(
    shared: &Shared,
    settings: &Settings,
    book: &Book,
    own_context: bool,
) -> Result<Tab> {
    let context = book.context.as_deref().filter(|_| own_context);
    let startup = || book.ledger.borrow_mut().startup(shared, &book.name);
    let Some(blank) = context.is_none().then(startup).flatten() else {
        return create(shared, settings, book, context, Hold::Owned(book.name.clone())).await;
    };
    // Left open when it cannot be used, not closed: it may be the launch's only page.
    shared.attach(settings, blank.clone()).await.inspect_err(|_| book.ledger.borrow_mut().release(&blank))
}

/// A new tab in `context`, held so from its first moment, then prepared, or
/// closed when it cannot be. In attach mode it opens in a window of its own,
/// behind the ones a person is using.
pub(crate) async fn create(
    shared: &Shared,
    settings: &Settings,
    book: &Book,
    context: Option<&str>,
    hold: Hold,
) -> Result<Tab> {
    let tab = new_tab(&shared.cdp, context, settings.attach.is_some()).await?;
    book.ledger.borrow_mut().hold(&tab.target, hold);
    if let Err(error) = prepare(settings, &tab, shared.identity.as_ref()).await {
        let _ = shared.cdp.close_target(&tab.target).await;
        return Err(error);
    }
    Ok(tab)
}

/// Before a command: take back what an earlier daemon left and the tabs this
/// session came to hold; before one that acts on a tab, reload crashed ones
/// and reopen kept ones that went away; each said once; let go of the earlier
/// contexts no tab of it lives in any more. Before one that may open or close
/// a tab, note what is open so [`follow`] can tell.
pub(crate) async fn begin(holder: &mut impl Holder, changing: bool, acting: bool) -> Vec<String> {
    let mut notes: Vec<String> = holder.book().notice.take().into_iter().collect();
    notes.extend(recover::resume(holder).await.ok().flatten());
    notes.extend(ledger::arrivals(holder).await);
    if acting {
        notes.extend(recover::crashed(holder).await);
        notes.extend(recover::restore(holder).await.ok().flatten());
    }
    if let Some(shared) = holder.host().running().await {
        let _ = holder.book().prune_contexts(&shared).await;
    }
    holder.book().before = None;
    if changing && holder.book().active.is_some() {
        holder.book().before = page_ids(&holder.host()).await.ok();
    }
    notes
}

/// This session's tab, opened on first use, and whether it is not the one it
/// was: a tab re-attached or opened anew has no document the session knows.
pub(crate) async fn current(holder: &mut impl Holder) -> Result<(Tab, bool)> {
    let host = holder.host();
    let shared = host.shared().await?;
    holder.book().reconcile(&shared).await?;
    if let Some(tab) = holder.book().live(&shared) {
        return Ok((tab, false));
    }
    let previous = holder.book().active.clone();
    if let Some(previous) = &previous
        && shared.cdp.target_exists(&previous.target).await?
    {
        return Ok((attach(holder, &previous.target).await?, true));
    }
    let book = holder.book();
    book.lost |= previous.is_some_and(|tab| !Rc::ptr_eq(&tab.cdp, &shared.cdp));
    let tab = fresh(&shared, host.settings(), book, true).await?;
    switch(holder, Some(tab.clone()));
    Ok((tab, true))
}

pub(crate) fn switch(holder: &mut impl Holder, tab: Option<Tab>) {
    let (left, next) = holder.book().switch(tab);
    holder.switched(left, next);
}

/// The active tab, now another target under the same handle: what the
/// session knew of its document is forgotten, not parked.
pub(crate) fn replace(holder: &mut impl Holder, tab: Tab) {
    let (left, _) = holder.book().switch(Some(tab));
    holder.switched(left.clone(), left);
}

pub(crate) async fn attach(holder: &mut impl Holder, target: &str) -> Result<Tab> {
    let host = holder.host();
    let shared = host.shared().await?;
    let tab = shared.attach(host.settings(), target.to_owned()).await?;
    switch(holder, Some(tab.clone()));
    Ok(tab)
}

/// Bring a tab to the front as the active one, attaching to it only when it is not already.
pub(crate) async fn select(holder: &mut impl Holder, target: &str) -> Result<Tab> {
    let shared = holder.host().shared().await?;
    let tab = match holder.book().live(&shared).filter(|tab| tab.target == target) {
        Some(tab) => tab,
        None => attach(holder, target).await?,
    };
    tab.activate().await?;
    Ok(tab)
}

/// A new tab in this session's context, made the active one.
pub(crate) async fn open(holder: &mut impl Holder) -> Result<Tab> {
    let host = holder.host();
    let shared = host.shared().await?;
    let book = holder.book();
    book.reconcile(&shared).await?;
    let hold = Hold::Owned(book.name.clone());
    let tab = create(&shared, host.settings(), book, book.context.as_deref(), hold).await?;
    switch(holder, Some(tab.clone()));
    Ok(tab)
}

/// After this session let go of a tab: when it was the active one, go back
/// to the one before, and say where that is.
pub(crate) async fn leave(holder: &mut impl Holder, target: &str) -> Result<Option<String>> {
    match holder.book().active.as_ref().is_some_and(|tab| tab.target == target) {
        true => Ok(fall_back(holder).await?.map(|(_, url)| url)),
        false => Ok(None),
    }
}

/// Once the active tab is gone: the one chosen by [`Book::next`] made the
/// active one, and where it is; none when this session has no tab open.
pub(crate) async fn fall_back(holder: &mut impl Holder) -> Result<Option<(Tab, String)>> {
    switch(holder, None);
    let open = pages(&holder.host()).await?;
    let ids = open.iter().map(|page| target_id(page).to_owned()).collect();
    let Some(target) = holder.book().next(&ids) else { return Ok(None) };
    let tab = attach(holder, &target).await?;
    let page = open.iter().find(|page| target_id(page) == target);
    Ok(Some((tab, page.and_then(|page| page["url"].as_str()).unwrap_or_default().to_owned())))
}

/// After an action: follow a tab it opened, or, when it closed this tab, go
/// back to the one this session was on before.
pub(crate) async fn follow(holder: &mut impl Holder, tab: &Tab) -> Result<(Tab, Vec<String>)> {
    let Some(before) = holder.book().before.take() else {
        return Ok((tab.clone(), Vec::new()));
    };
    let open = pages(&holder.host()).await?;
    // The tab list is the whole browser's and another session may open a tab
    // concurrently: only this opener's new targets can be adopted.
    if let Some(newest) = open
        .iter()
        .find(|page| !before.contains(target_id(page)) && page["openerId"].as_str() == Some(&tab.target))
    {
        let url = crate::web::shown(newest["url"].as_str().unwrap_or_default());
        let tab = attach(holder, target_id(newest)).await?;
        let name = holder.book().handle(&tab.target);
        return Ok((tab, vec![format!("A new tab {name} opened at {url} and is now the active one.")]));
    }
    if open.iter().any(|page| target_id(page) == tab.target) {
        return Ok((tab.clone(), Vec::new()));
    }
    match fall_back(holder).await? {
        Some((tab, url)) => {
            Ok((tab, vec![format!("This tab closed; switched to {}.", crate::web::shown(&url))]))
        }
        None => Err(Error::Tool("the tab closed during the action and this session has no other tab".into())),
    }
}
