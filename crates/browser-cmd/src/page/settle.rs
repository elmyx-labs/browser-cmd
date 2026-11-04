//! Waiting on a page: for it to go quiet after an action, and for text or an
//! address to arrive.

use std::time::Duration;

use serde_json::Value;

use crate::chrome::tab::Tab;
use crate::page::canvas::LARGE_CANVAS;
use crate::page::frames::Epoch;
use crate::page::js::QUIET;
use crate::page::reading::read;
use crate::page::view::{Area, capture};
use crate::page::world::World;
use crate::settings::Waits;
use crate::{Error, Result};

/// The share of the DOM's nodes that may change per 100 ms and still count as calm.
const CALM_RATIO: f64 = 0.01;
/// Consecutive calm looks that make a page settled.
const CALM_SAMPLES: u32 = 2;
/// Consecutive calm looks that accept a page with nothing to read yet — a
/// canvas, a viewer, a blank document — rather than holding it to the cap.
const EMPTY_SAMPLES: u32 = 10;
/// How old a document must be before it counts: a client-side redirect fires
/// within this of a load.
const REDIRECT_GUARD_MS: f64 = 1000.0;
/// How long a canvas gets to stop redrawing; an animation never stops.
const DRAW_BUDGET: Duration = Duration::from_secs(3);
/// Between two looks at a canvas: about a dozen frames, enough for a tile to land.
const DRAW_INTERVAL: Duration = Duration::from_millis(200);
/// Image pixels per CSS pixel in a look: enough to see a change, cheap to take.
const DRAW_SCALE: f64 = 0.125;

/// How a page settled: `capped` by the bound it was held to, when it never went quiet.
#[derive(Clone, Copy)]
pub(crate) struct Settled {
    pub(crate) capped: Option<Duration>,
}

/// Wait for the document to be loaded, to have something on it, its DOM to have
/// stopped changing, and — when the Network domain is on — its own requests to
/// have answered, bounded. A page that renders itself after load — most of
/// them — is otherwise snapshotted half-drawn.
pub(crate) async fn settle(world: &mut World, tab: &Tab, waits: &Waits) -> Settled {
    let deadline = tokio::time::Instant::now() + waits.settle;
    // The first look measures the time before the action as well, which would
    // dilute what the action changed: it only starts the count.
    let mut calm: Option<u32> = None;
    loop {
        let state = world.call(tab, QUIET, Vec::new()).await.unwrap_or(Value::Null);
        calm = Some(match (calm, state["ratio"].as_f64()) {
            (Some(calm), Some(ratio)) if ratio <= CALM_RATIO => calm + 1,
            _ => 0,
        });
        if settled(&state, calm.unwrap_or_default())
            && tab.cdp.events().navigation_quiet(&tab.session, waits.quiet)
            && tab.cdp.events().network_quiet(&tab.session)
        {
            if state["canvas"].as_f64().is_some_and(|share| share >= LARGE_CANVAS) {
                painted(world, tab).await;
            }
            return Settled { capped: None };
        }
        if tokio::time::Instant::now() >= deadline {
            return Settled { capped: Some(waits.settle) };
        }
        tokio::time::sleep(waits.poll).await;
    }
}

/// A canvas redraws without touching the DOM — a map fetching its tiles, a
/// chart easing in — so a quiet DOM says nothing about it. Two equal small
/// pictures in a row do, bounded so that an animation never holds an action.
async fn painted(world: &mut World, tab: &Tab) {
    let deadline = tokio::time::Instant::now() + DRAW_BUDGET;
    let mut last = None;
    while tokio::time::Instant::now() < deadline {
        let Ok(look) = capture(world, tab, Area::Viewport, DRAW_SCALE).await else { return };
        if last.as_ref() == Some(&look) {
            return;
        }
        last = Some(look);
        tokio::time::sleep(DRAW_INTERVAL).await;
    }
}

fn settled(state: &Value, calm: u32) -> bool {
    state["ready"].as_str().is_some_and(|ready| ready != "loading")
        && calm >= CALM_SAMPLES
        && (state["content"] == true || calm >= EMPTY_SAMPLES)
        && state["age"].as_f64().is_some_and(|age| age >= REDIRECT_GUARD_MS)
}

/// Wait until `text` is on the page, `gone` no longer is, and the URL contains
/// `url`, whichever were asked for. `Ok(false)` is a deadline reached with the
/// conditions observed not to hold; a deadline reached while they could not be
/// decided is an error that says so.
pub(crate) async fn wait_for(
    world: &mut World,
    tab: &Tab,
    waits: &Waits,
    text: Option<&str>,
    gone: Option<&str>,
    url: Option<&str>,
    limit: Duration,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + limit;
    let mut unknown = true;
    loop {
        // Only read bodies when a body condition was asked for: a URL-only wait
        // must not depend on whether an unrelated frame is readable.
        let check = async {
            let epoch = Epoch::of(tab);
            let reading = match text.is_some() || gone.is_some() {
                true => Some(read(world, tab, false, None, false, None).await?),
                false => None,
            };
            let at = match url {
                Some(_) => Some(tab.url().await?),
                None => None,
            };
            epoch.check(tab)?;
            let mut conditions = Vec::new();
            if let Some(reading) = &reading {
                conditions.extend(text.map(|text| reading.contains(text)));
                conditions.extend(gone.map(|gone| reading.contains(gone).map(|found| !found)));
            }
            if let (Some(at), Some(url)) = (&at, url) {
                conditions.push(Some(at.contains(url)));
            }
            Ok(all(&conditions))
        };
        match tokio::time::timeout_at(deadline, check).await {
            Ok(Ok(Some(true))) => return Ok(true),
            Ok(Ok(Some(false))) => unknown = false,
            Ok(Ok(None)) | Ok(Err(Error::DocumentChanged)) => unknown = true,
            Ok(Err(error)) => return Err(error),
            Err(_) => break,
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + waits.poll).min(deadline)).await;
    }
    match unknown {
        true => Err(Error::Tool(
            "wait could not decide its conditions before the deadline: reading was incomplete or documents were \
             changing"
                .into(),
        )),
        false => Ok(false),
    }
}

/// Conjunction over observed true/false and unknown, without converting
/// unavailable content into false or treating an unknown as a success.
fn all(conditions: &[Option<bool>]) -> Option<bool> {
    if conditions.contains(&Some(false)) {
        Some(false)
    } else if conditions.contains(&None) {
        None
    } else {
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{all, settled};
    use serde_json::json;

    #[test]
    fn a_page_settles_only_calm_loaded_old_enough_and_with_something_on_it() {
        let page = json!({"ready":"complete","content":true,"age":5000.0});
        assert!(settled(&page, 2));
        assert!(!settled(&page, 1), "one calm look is not two");
        assert!(!settled(&json!({"ready":"loading","content":true,"age":5000.0}), 9));
        assert!(!settled(&json!({"ready":"complete","content":true,"age":300.0}), 9), "may still redirect");
        let empty = json!({"ready":"interactive","content":false,"age":5000.0});
        assert!(!settled(&empty, 2) && settled(&empty, 10), "an empty page waits longer, not forever");
        assert!(!settled(&serde_json::Value::Null, 10));
    }

    #[test]
    fn an_unknown_condition_is_neither_a_pass_nor_a_failure() {
        assert_eq!(all(&[Some(true), None]), None);
        assert_eq!(all(&[Some(false), None]), Some(false));
        assert_eq!(all(&[Some(true), Some(true)]), Some(true));
    }
}
