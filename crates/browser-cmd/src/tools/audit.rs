//! `tab check`: every tab of this session looked at now — open, loaded, an
//! error page, a refusal, a wall, empty, still at its kept address, closed or
//! crashed — reopening nothing, with a picture of each kept one, and a
//! one-line verdict on the kept ones.

use std::rc::Rc;

use serde_json::Value;

use crate::browser::host::Shared;
use crate::chrome::tab::Tab;
use crate::page::view::Area;
use crate::page::world::World;
use crate::session::Session;
use crate::tabs::status::TabStatus;
use crate::tabs::target_id;
use crate::text::fence::untrusted;
use crate::tools::tabs::{address, line, others};
use crate::{Error, Result};

pub(crate) async fn check(session: &mut Session, tabs: &[Value]) -> Result<String> {
    let shared = session.host.shared().await?;
    let mut entries = session.book.registry.entries.clone();
    entries.sort_by_key(|entry| entry.kept.is_none());
    let (mut lines, mut faults, mut pictured) = (Vec::new(), Vec::new(), false);
    for entry in &entries {
        let page = tabs.iter().find(|tab| target_id(tab) == entry.target);
        let Some(page) = page.filter(|page| page["crashed"] != true) else {
            let lost = match page {
                Some(_) => "crashed".into(),
                None if entry.gone() => entry.status.to_string(),
                None => "closed (was kept)".into(),
            };
            lines.push(format!(" {} {lost}", entry.name()));
            let fault = lost.trim_end_matches(" (was kept)");
            faults.extend(entry.kept.as_ref().map(|_| format!("{} {fault}", entry.name())));
            continue;
        };
        let (url, title) = (address(entry, page).to_owned(), page["title"].as_str().unwrap_or_default());
        let mut shot = String::new();
        match probe(session, &shared, &entry.target, entry.kept.is_some()).await {
            Ok((now, picture)) => {
                session.book.registry.seen(&entry.target, &url, title, now);
                pictured |= picture.is_some();
                shot = picture.map_or(String::new(), |path| format!(" [screenshot {path}]"));
            }
            Err(error) => session.book.mark(&entry.target, TabStatus::Unreadable(error.to_string())),
        }
        let Some(entry) = session.book.registry.of(&entry.target) else { continue };
        lines.push(format!("{}{shot}", line(session, entry, page)));
        faults.extend(entry.fault().map(|fault| format!("{} {fault}", entry.name())));
    }
    if pictured && let Some(tab) = &session.book.active {
        let _ = tab.activate().await;
    }
    lines.extend(others(session, tabs));
    let kept = entries.iter().filter(|entry| entry.kept.is_some()).count();
    let verdict = match (kept, faults.len()) {
        (0, _) => "No tab is kept.".to_owned(),
        (_, 0) => format!("{kept} kept tab{}: all ok.", plural(kept)),
        (_, bad) => {
            format!("{kept} kept tab{}: {} ok, {bad} not ({}).", plural(kept), kept - bad, faults.join(", "))
        }
    };
    Ok(match lines.is_empty() {
        true => format!("{verdict} This session has no tabs open."),
        false => format!("{verdict}\n{}", untrusted(&lines.join("\n"))),
    })
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// A tab's status read live, and for a kept one a picture of its viewport,
/// brought to the front first so it is drawn.
async fn probe(
    session: &mut Session,
    shared: &Rc<Shared>,
    target: &str,
    kept: bool,
) -> Result<(TabStatus, Option<String>)> {
    let tab = session.book.reach(shared, session.settings(), target).await?;
    let mut world = World::new();
    let now = crate::tabs::status::read(&tab, &mut world, session.settings()).await?;
    let picture = match kept {
        true => Some(
            picture(session, &tab, &mut world).await.unwrap_or_else(|error| format!("not taken: {error}")),
        ),
        false => None,
    };
    Ok((now, picture))
}

async fn picture(session: &mut Session, tab: &Tab, world: &mut World) -> Result<String> {
    tab.activate().await?;
    let data = crate::page::view::capture(world, tab, Area::Viewport, 1.0).await?;
    let bytes = crate::text::base64::unbase64(&data)
        .ok_or_else(|| Error::Tool("the browser returned no image".into()))?;
    let path = session.output.save_shot(session.host.settings(), &bytes)?;
    Ok(session.relative(&path))
}
