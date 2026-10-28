//! What a page shows that no tree holds: its canvases, the text they draw, and
//! a picture of a page with nothing the tree can name.

use crate::chrome::tab::Tab;
use crate::page::canvas;
use crate::session::{Session, notes};

/// The notes about what the page draws. `empty` is whether the state named no control.
pub(crate) async fn visual(session: &mut Session, tab: &Tab, empty: bool) -> Vec<String> {
    let mut notes = Vec::new();
    // A canvas draws what no tree or text holds. On a new document, or when
    // the canvases change, the agent is told where they are; ones that fill
    // much of the view are also pictured.
    let loader = session.document.loader.clone();
    let new_document = std::mem::replace(&mut session.document.seen_load, loader.clone()) != loader;
    let found = canvas::canvases(&mut session.document.world, tab).await;
    let kinds = found.as_ref().map(|(_, kinds, _)| kinds.clone());
    let changed = std::mem::replace(&mut session.document.canvases, kinds.clone()) != kinds;
    let canvases = match (new_document || changed, found) {
        (true, Some((place, _, coverage))) => {
            let told = !session.document.told.insert("canvas");
            // The lack of WebGL is said only in the full note, so only then asked.
            let blank = !told
                && canvas::webgl_renderer(&mut session.document.world, tab)
                    .await
                    .is_ok_and(|renderer| renderer.is_null());
            Some((notes::canvas(&place, coverage, told, blank), coverage))
        }
        _ => None,
    };
    let large = canvases.as_ref().is_some_and(|(_, coverage)| *coverage >= canvas::LARGE_CANVAS);
    notes.extend(canvases.map(|(note, _)| note));
    if session.settings().canvas_text {
        let limit = session.settings().limits.new_text;
        let drawn = canvas::drawn(tab, &mut session.document.drawn, new_document).await;
        notes.extend(drawn.and_then(|drawn| notes::drawn(&drawn, limit)));
    }
    // A page with nothing the tree can name — a canvas, a game, a map — is
    // blind text; a picture is the only account of it, so one is taken and
    // said, saving the agent a screenshot it would always have to make next.
    if session.output.picture.is_none()
        && (empty || large)
        && let Some(path) =
            session.output.glance(&mut session.document.world, tab, session.host.settings()).await
    {
        let relative = session.relative(&path);
        notes.push(match session.document.told.insert("glance") {
            true => format!(
                "No controls were captured yet; the page may still be rendering or use visual content. A \
                 screenshot of the viewport is saved to {relative}."
            ),
            false => format!("No controls captured; viewport screenshot: {relative}."),
        });
    }
    notes
}
