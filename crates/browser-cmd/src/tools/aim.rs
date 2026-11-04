//! Aiming at what a ref, a visible name or a point addresses, so a click lands on it and nothing covering it.

use serde_json::json;

use crate::chrome::tab::Tab;
use crate::input::rect::Rect;
use crate::page::frames::Epoch;
use crate::page::js::{CLICKABLE, COVERED_BY, LOCATE};
use crate::page::view::{Quad, box_of};
use crate::session::Session;
use crate::tools::args::Target;
use crate::{Error, Result};

pub(crate) async fn point(
    session: &mut Session,
    tab: &Tab,
    target: &Target,
    verb: &str,
) -> Result<(f64, f64)> {
    target.validate()?;
    // A name asks for the current control; a handle asks for one specific
    // observed node. Refresh names before aiming, never replay a click.
    if target.r#ref.as_deref().is_some_and(|name| !crate::page::snapshot::is_handle(name)) {
        let epoch = Epoch::of(tab);
        session.document.state(tab, false, true).await?;
        epoch.check(tab)?;
    }
    match (&target.r#ref, target.x, target.y) {
        (Some(reference), _, _) => match session.node(reference) {
            Ok(node) => {
                let aimed = match find(session, tab, node).await {
                    Ok(Ok(rect)) => aim_in(session, tab, node, rect).await,
                    // Replaced since the state was read, as pages that render
                    // twice do. Never rebound silently: the page is read again
                    // and the element of the same role and name, if one, named.
                    Ok(Err(stale)) => {
                        session.document.state(tab, false, true).await?;
                        Err(Error::Tool(match session.document.refs.renamed(reference) {
                            Some(now) => format!("{stale}; the page re-rendered it, and it is now {now}"),
                            None => stale,
                        }))
                    }
                    Err(error) => Err(error),
                };
                aimed.map_err(|error| Error::Tool(format!("cannot {verb} {reference}: {error}")))
            }
            // A name not in the accessibility tree may be a role-less custom
            // control; resolve it to a node so it is aimed and cover-checked
            // like any ref.
            Err(missing) if crate::page::snapshot::is_handle(reference) => Err(missing),
            Err(missing) => match clickable_by_text(session, tab, reference).await? {
                Some(node) => aim(session, tab, node).await,
                None => Err(missing),
            },
        },
        (None, Some(x), Some(y)) => Ok((x, y)),
        _ => Err(Error::Tool(format!("{verb} needs a ref, or x and y"))),
    }
}

/// A point on the element that a click will actually reach.
pub(crate) async fn aim(session: &mut Session, tab: &Tab, node: i64) -> Result<(f64, f64)> {
    let rect = locate(session, tab, node).await?;
    aim_in(session, tab, node, rect).await
}

async fn aim_in(session: &mut Session, tab: &Tab, node: i64, rect: Rect) -> Result<(f64, f64)> {
    let (width, height) = crate::page::view::viewport(&mut session.document.world, tab).await?;
    let rect = rect
        .visible(width, height)
        .ok_or_else(|| Error::Tool("the element has no visible area in the viewport".into()))?;
    let (x, y) = session.hand.point_in(rect);
    if let Some(cover) = covered_by(session, tab, node, x, y).await? {
        return Err(Error::Tool(format!(
            "element is covered by {cover}; clear it first — click it, press Escape, scroll, \
             or take a fresh snapshot to find it (a pop-up that arrived after the last one has no ref yet)"
        )));
    }
    Ok((x, y))
}

/// Resolve a role-less control without writing observable DOM markers.
async fn clickable_by_text(session: &mut Session, tab: &Tab, text: &str) -> Result<Option<i64>> {
    let found = session.document.world.node_result(tab, None, CLICKABLE, vec![json!(text)]).await?;
    if let Some(detail) = found.detail {
        return Err(Error::Tool(format!("{text:?} matches several clickable things: {detail}")));
    }
    Ok(found.node)
}

/// What sits at a point instead of the node, or `None`. Asked when aiming and
/// again after the pointer travelled: a live page can reflow the target out
/// from under the cursor in between.
pub(crate) async fn covered_by(
    session: &mut Session,
    tab: &Tab,
    node: i64,
    x: f64,
    y: f64,
) -> Result<Option<String>> {
    let hit =
        session.document.world.node_result(tab, Some(node), COVERED_BY, vec![json!(x), json!(y)]).await?;
    let Some(node) = hit.node else { return Ok(None) };
    let text = hit.detail.unwrap_or_default();
    Ok(Some(match session.document.refs.handle_for(node) {
        Some(handle) => format!("@{handle} {text:?}"),
        None => format!("{text:?}"),
    }))
}

pub(crate) async fn locate(session: &mut Session, tab: &Tab, node: i64) -> Result<Rect> {
    find(session, tab, node).await?.map_err(Error::Tool)
}

/// The element's box once brought into view, or what to say when the page
/// replaced it since the state that named it.
async fn find(session: &mut Session, tab: &Tab, node: i64) -> Result<std::result::Result<Rect, String>> {
    // Bring it into view and check it is really there, in its own frame.
    let seen = session.document.world.call_on(tab, node, LOCATE, Vec::new()).await?;
    match seen["error"].as_str() {
        Some(stale) if seen["stale"] == true => return Ok(Err(stale.to_owned())),
        Some(error) => return Err(Error::Tool(error.to_owned())),
        None => {}
    }
    // In the top frame's coordinates, so a control in a frame is clicked where it is.
    let rect = box_of(tab, node, Quad::Content).await?.ok_or_else(|| {
        Error::Tool(
            "the element has no box — no size or position, so it is hidden or has left the layout. \
             Take a fresh snapshot and act on what is visible."
                .into(),
        )
    })?;
    Ok(Ok(rect))
}
