//! Scrolling: a wheel turns whatever is under the pointer, to an edge, or to
//! the text asked for.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::js::SCROLL_TO_TEXT;
use crate::session::{Session, after};
use crate::tools::aim::{locate, point};
use crate::tools::args::{Target, args};
use crate::{Error, Result};

pub(crate) async fn scroll(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args {
        #[serde(flatten)]
        target: Target,
        direction: Option<String>,
        amount: Option<u32>,
        /// "bottom" or "top" scrolls until the page stops growing; other text is scrolled to.
        to: Option<String>,
    });
    let tab = session.current().await?;
    if let Some(to) = args.to.as_deref() {
        return match to {
            "top" | "bottom" => scroll_to_edge(session, &tab, &args.target, to).await,
            text => scroll_to_text(session, &tab, text).await,
        };
    }
    // A ref on its own means "bring this into view"; a ref with a
    // direction means "scroll the thing this sits in", which is the only
    // way to reach a panel, a sidebar or a chat: a wheel turns whatever is
    // under the pointer, and the page is merely what is under the middle.
    if let Some(reference) = &args.target.r#ref
        && args.direction.is_none()
    {
        let node = session.node(reference)?;
        locate(session, &tab, node)
            .await
            .map_err(|error| Error::Tool(format!("cannot scroll to {reference}: {error}")))?;
        return after(session, &tab, &format!("Scrolled {reference} into view")).await;
    }
    let limits = &session.settings().limits;
    let amount = f64::from(args.amount.unwrap_or(limits.scroll_default).clamp(1, limits.scroll_max));
    let direction = args.direction.as_deref().unwrap_or("down");
    let (dx, dy) = match direction {
        "up" => (0.0, -amount),
        "down" => (0.0, amount),
        "left" => (-amount, 0.0),
        "right" => (amount, 0.0),
        _ => return Err(Error::Tool("scroll direction must be up, down, left, or right".into())),
    };
    let ((x, y), aimed) = wheel_at(session, &tab, &args.target).await?;
    let before = crate::page::view::reach(&mut session.document.world, &tab, aimed.then_some((x, y))).await;
    session.hand.move_to(&tab, x, y).await?;
    session.hand.wheel(&tab, dx, dy).await?;
    // The wheel is acknowledged before the page has scrolled and repainted;
    // reading the position now would under-report a smooth scroll.
    session.settle(&tab).await;
    let reached = crate::page::view::reach(&mut session.document.world, &tab, aimed.then_some((x, y))).await;
    let over = match (&args.target.r#ref, args.target.x) {
        (Some(reference), _) => format!(" over {reference}"),
        (None, Some(_)) => format!(" over {x}, {y}"),
        _ => String::new(),
    };
    let moved = travelled(&before, &reached, direction);
    after(session, &tab, &format!("Scrolled {direction} {amount} px{over}. {moved}")).await
}

/// So a feed is collected in one call instead of a guessed number of scrolls.
/// Stops when neither position nor height changed for two passes, or at a pass ceiling.
async fn scroll_to_edge(session: &mut Session, tab: &Tab, target: &Target, edge: &str) -> Result<String> {
    let down = edge == "bottom";
    let ((x, y), aimed) = wheel_at(session, tab, target).await?;
    let step = f64::from(session.settings().limits.scroll_max);
    let dy = if down { step } else { -step };
    session.hand.move_to(tab, x, y).await?;
    let mut last = (i64::MIN, i64::MIN);
    let mut still = 0;
    let mut passes = 0;
    let ceiling = session.settings().limits.scroll_passes;
    while passes < ceiling {
        session.hand.wheel(tab, 0.0, dy).await?;
        session.settle(tab).await;
        let reach = crate::page::view::reach(&mut session.document.world, tab, aimed.then_some((x, y))).await;
        let here = (reach["top"].as_i64().unwrap_or(0), reach["max"].as_i64().unwrap_or(0));
        passes += 1;
        if here == last {
            still += 1;
            if still >= 2 {
                break;
            }
        } else {
            still = 0;
        }
        last = here;
    }
    let (top, max) = last;
    let ended = if down { top >= max } else { top <= 0 };
    let how = if ended {
        format!("reached the {edge}")
    } else {
        format!("stopped at {top} of {max} after {passes} passes (the page kept growing)")
    };
    after(session, tab, &format!("Scrolled to the {edge}: {how}")).await
}

/// Where the wheel turns: over what the caller named, and over the middle of
/// the page when it named nothing — and whether it named anything. A point is
/// the only handle some regions have: a live list recycles its nodes between
/// one call and the next, and a ref to a message already redrawn names nothing.
async fn wheel_at(session: &mut Session, tab: &Tab, target: &Target) -> Result<((f64, f64), bool)> {
    let at = match (&target.r#ref, target.x, target.y) {
        (None, None, None) => {
            let (width, height) = crate::page::view::viewport(&mut session.document.world, tab).await?;
            (width / 2.0, height / 2.0)
        }
        _ => point(session, tab, target, "scroll over").await?,
    };
    Ok((at, target.r#ref.is_some() || target.x.is_some()))
}

/// Bring the first element bearing this text into view, so a thing far down
/// a long page is reached in one move instead of a scroll-and-look loop.
/// The text must be in the DOM: a virtualized list that has not rendered
/// the row yet holds nothing to find (scroll `to:"bottom"` loads it first).
pub(crate) async fn scroll_to_text(session: &mut Session, tab: &Tab, text: &str) -> Result<String> {
    let found = session.document.world.call(tab, SCROLL_TO_TEXT, vec![json!(text)]).await?;
    if found != true {
        return Err(Error::Tool(format!(
            "no text like {text:?} is on the page now; it may be further down a list that loads as \
             you go (scroll to:\"bottom\" first) or not present"
        )));
    }
    after(session, tab, &format!("Scrolled {text:?} into view")).await
}

/// What the wheel actually did, said plainly enough to end a loop. A frame
/// from another document cannot be measured from here, and says so rather than
/// reporting a zero that reads like an answer.
fn travelled(before: &Value, after: &Value, direction: &str) -> String {
    let sideways = matches!(direction, "left" | "right");
    let (now, was) = match sideways {
        true => (&after["left"], &before["left"]),
        false => (&after["top"], &before["top"]),
    };
    let reach = match sideways {
        true => &after["maxLeft"],
        false => &after["max"],
    };
    match after["kind"].as_str() {
        Some("frame") => "The pointer is over another document, which this page cannot measure: \
                          what moved is in the state."
            .to_owned(),
        Some("region" | "page") => {
            let (now, was, reach) = (
                now.as_f64().unwrap_or_default(),
                was.as_f64().unwrap_or_default(),
                reach.as_f64().unwrap_or_default(),
            );
            let what = if after["kind"] == "region" { "This region" } else { "The page" };
            let ends = |place: &str| format!("{what} did not move: it is already at the {place}.");
            match (now - was, now, reach) {
                (0.0, at, _) if at <= 0.0 => ends(if sideways { "left" } else { "top" }),
                (0.0, at, reach) if at >= reach => ends(if sideways { "right" } else { "bottom" }),
                (0.0, ..) => format!("{what} did not move."),
                (by, at, reach) => {
                    format!("{what} moved {by:.0} px, and is at {at:.0} of {reach:.0}.")
                }
            }
        }
        _ => "Nothing under the pointer scrolls.".to_owned(),
    }
}
