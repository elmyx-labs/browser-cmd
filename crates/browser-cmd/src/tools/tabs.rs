//! The tab commands: this session's tabs by handle — listing, switching,
//! opening and closing them, keeping the ones that hold evidence and closing
//! the rest, reopening kept ones an earlier run left gone, borrowing a page
//! nobody holds and handing one to another session — and closing the session's own.

use serde_json::{Value, json};

use crate::session::{Session, after};
use crate::tabs::registry::{Entry, same_page};
use crate::tabs::status::TabStatus;
use crate::tabs::target_id;
use crate::text::fence::untrusted;
use crate::tools::args::{Empty, args, parse};
use crate::tools::navigate::go;
use crate::web::web_url;
use crate::{Error, Result};

pub(crate) async fn tab(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args {
        action: String,
        id: Option<String>,
        url: Option<String>,
        label: Option<String>,
        to: Option<String>,
    });
    let tabs = crate::tabs::survey(session).await?;
    match args.action.as_str() {
        "list" => Ok(list(session, &tabs)),
        "check" => crate::tools::audit::check(session, &tabs).await,
        "tidy" => {
            let (closed, kept, switched) = crate::tabs::close::tidy(session, &tabs).await?;
            let mut said = match closed.is_empty() {
                true => format!("Nothing to close; open and kept: {}.", kept.join(", ")),
                false => format!("Closed {}; open and kept: {}.", closed.join(", "), kept.join(", ")),
            };
            said.extend(switched.map(|name| format!(" Switched to {name}.")));
            Ok(said)
        }
        "keep" => {
            let target = session.book.chosen(&tabs, args.id.as_deref())?;
            let tab = tabs.iter().find(|tab| target_id(tab) == target);
            let url = tab.and_then(|tab| tab["url"].as_str()).unwrap_or_default();
            let entry = session.book.registry.keep(&target, url, args.label);
            Ok(format!("Kept {} at {}.", entry.name(), crate::web::shown(url)))
        }
        "unkeep" => {
            let target = session.book.chosen(&tabs, args.id.as_deref())?;
            Ok(format!("{} is no longer kept.", session.book.registry.unkeep(&target).name()))
        }
        "switch" => {
            let target = session.book.chosen(&tabs, args.id.as_deref())?;
            let tab = crate::tabs::select(session, &target).await?;
            let name = session.book.handle(&target);
            after(session, &tab, &format!("Switched to {name}")).await
        }
        "close" => {
            let target = session.book.chosen(&tabs, args.id.as_deref())?;
            let said = match session.book.registry.of(&target).is_some_and(|entry| entry.borrowed) {
                true => "Gave the borrowed tab back, open",
                false => "Closed the tab",
            };
            Ok(match crate::tabs::close::close(session, &target).await? {
                Some(url) => format!("{said}; switched to {}.", crate::web::shown(&url)),
                None => format!("{said}."),
            })
        }
        "use" => {
            let word =
                args.id.ok_or_else(|| Error::Tool("tab use needs a number or id from tab list".into()))?;
            let tab = crate::tabs::ledger::borrow(session, &tabs, &word).await?;
            let entry = session.book.registry.adopt(&tab.target);
            let said =
                format!("{} {}", if entry.borrowed { "Borrowed" } else { "Switched to" }, entry.name());
            after(session, &tab, &said).await
        }
        "give" => {
            let target = session.book.chosen(&tabs, args.id.as_deref())?;
            let to = args
                .to
                .filter(|to| !to.is_empty())
                .ok_or_else(|| Error::Tool("tab give needs to=SESSION".into()))?;
            let name = session.book.handle(&target);
            Ok(match crate::tabs::ledger::give(session, &target, &to).await? {
                Some(url) => format!("Gave {name} to session {to}; switched to {}.", crate::web::shown(&url)),
                None => format!("Gave {name} to session {to}."),
            })
        }
        "restore" => crate::tabs::recover::bring_back(session, args.id.as_deref()).await,
        "new" => {
            let url = args.url.as_deref().map(web_url).transpose()?;
            let tab = crate::tabs::open(session).await?;
            if let Some(url) = url {
                go(session, &tab, url.as_str()).await?;
            }
            let name = session.book.handle(&tab.target);
            after(session, &tab, &format!("Opened tab {name}")).await
        }
        other => Err(Error::Tool(format!(
            "tab action must be list, check, tidy, keep, unkeep, switch, new, close, restore, use or give, not \
             {other}"
        ))),
    }
}

/// This session's open tabs and kept ones closed or gone, one line each,
/// then the other sessions' as a count, then the ones nobody holds.
fn list(session: &Session, tabs: &[Value]) -> String {
    let mut lines: Vec<String> = session
        .book
        .registry
        .entries
        .iter()
        .filter_map(|entry| match tabs.iter().find(|tab| target_id(tab) == entry.target) {
            Some(tab) => Some(line(session, entry, tab)),
            None => entry.kept.as_ref().map(|_| {
                let mut entry = entry.clone();
                if !entry.gone() {
                    entry.status = TabStatus::Closed;
                }
                line(session, &entry, &json!({ "title": entry.title }))
            }),
        })
        .collect();
    lines.extend(others(session, tabs));
    match lines.is_empty() {
        true => "This session has no tabs open.".into(),
        false => untrusted(&lines.join("\n")),
    }
}

/// `*t3 kept "label" ok https://… — Title (moved from …)`, the star on the
/// active tab, where it is now.
pub(crate) fn line(session: &Session, entry: &Entry, tab: &Value) -> String {
    let active = session.book.active.as_ref().is_some_and(|tab| tab.target == entry.target);
    let kept = if entry.kept.is_some() { " kept" } else { "" };
    let borrowed = if entry.borrowed { " borrowed" } else { "" };
    let label = entry.label.as_ref().map_or(String::new(), |label| format!(" {}", json!(label)));
    let at = address(entry, tab);
    let moved = match entry.kept.as_deref().filter(|kept| !same_page(kept, at)) {
        Some(kept) => format!(" (moved from {})", crate::web::shown(kept)),
        None => String::new(),
    };
    format!(
        "{}{}{kept}{borrowed}{label} {} {} — {}{moved}",
        if active { "*" } else { " " },
        entry.name(),
        entry.status,
        crate::web::shown(at),
        tab["title"].as_str().unwrap_or_default()
    )
}

/// The open tabs that are not this session's: other sessions' as one count
/// with their names, then the ones nobody holds, numbered for `tab use`.
pub(crate) fn others(session: &Session, tabs: &[Value]) -> Vec<String> {
    let (mut sessions, unowned) = crate::tabs::ledger::strangers(tabs, &session.book);
    let mut lines = Vec::new();
    if !sessions.is_empty() {
        let count = sessions.len();
        sessions.sort();
        sessions.dedup();
        let (tabs, whose) = (if count == 1 { "" } else { "s" }, if sessions.len() == 1 { "" } else { "s" });
        lines.push(format!("{count} other tab{tabs} open, of session{whose} {}", sessions.join(", ")));
    }
    if !unowned.is_empty() {
        lines.push("Not any session's; `tab use N` borrows one:".into());
    }
    lines.extend(unowned.iter().enumerate().map(|(n, tab)| {
        let id = target_id(tab).get(..8).unwrap_or(target_id(tab));
        let (url, title) =
            (tab["url"].as_str().unwrap_or_default(), tab["title"].as_str().unwrap_or_default());
        format!(" {} {id} {} — {title}", n + 1, crate::web::shown(url))
    }));
    lines
}

/// Where a tab is; while a page it was sent to has not begun, where it was sent.
pub(crate) fn address<'a>(entry: &'a Entry, tab: &'a Value) -> &'a str {
    tab["url"].as_str().filter(|url| !url.is_empty()).unwrap_or(&entry.url)
}

/// End this session; the browser itself stays warm for the next one.
pub(crate) async fn close(session: &mut Session, args: Value) -> Result<String> {
    parse::<Empty>(args)?;
    let ended = crate::tabs::close::end(session).await?;
    let mut said = Vec::new();
    said.extend((!ended.closed.is_empty()).then(|| format!("Closed {}.", ended.closed.join(", "))));
    said.extend((!ended.given.is_empty()).then(|| format!("Gave back {}, open.", ended.given.join(", "))));
    said.extend((!ended.kept.is_empty()).then(|| format!("Left kept {} open.", ended.kept.join(", "))));
    match session.book.dispose_contexts(true, |entry| entry.kept.is_some()).await {
        Ok(true) => said.push("Closed this session's browser context.".into()),
        Ok(false) => {}
        Err(error) => {
            return Err(Error::Tool(format!(
                "{}browser context disposal was not confirmed; context identity retained: {error}",
                if ended.closed.len() + ended.given.len() == 0 { "" } else { "Tabs closed, but " }
            )));
        }
    }
    if said.is_empty() {
        said.push("This session had no tab open.".into());
    }
    session.ended = true;
    said.push("The session ended; the browser and its profile stay for the next one.".into());
    Ok(said.join(" "))
}
