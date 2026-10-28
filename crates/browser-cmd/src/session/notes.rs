//! What an action's answer says, in words, about what happened on the page.

use serde_json::Value;

use crate::chrome::events::Note;
use crate::page::overlay::Overlay;
use crate::page::settle::Settled;
use crate::page::walls::Wall;
use crate::session::wall::{Passed, Stand};
use crate::text::{listed, sentence};

/// What a tab volunteered while nobody was asking.
pub(crate) fn event(note: &Note) -> String {
    match note {
        Note::Dialog { kind, message, accepted } => format!(
            "The page opened a {kind} dialog saying {message:?}; it was {}. A dialog must be answered at \
             once or the page stops, so this is the answer the host chose in advance, not a \
             decision made here.",
            if *accepted { "accepted" } else { "dismissed" }
        ),
        Note::Download { completed, record } => {
            format!(
                "Chrome reported download {}: {record}",
                if *completed { "completed" } else { "canceled" }
            )
        }
        Note::Failed { kind, reason } => format!("A {kind} request failed: {reason}."),
        Note::Refused { url, status } => format!("The page asked for {url} and got {status}."),
        Note::Crashed => "The page crashed: its renderer died and what it showed is gone. Navigate to it \
                          again; if it crashes again it is too heavy for this browser, so take another \
                          route to the answer."
            .into(),
    }
}

/// The warning for the action line when settling hit its cap.
pub(crate) fn capped(settled: Settled) -> Option<String> {
    let cap = settled.capped?;
    Some(format!("[warning] page kept changing; settled after {:.1} s cap", cap.as_secs_f64()))
}

/// A 403 or 404 renders as a page, and "Access denied" in a heading looks like
/// content, not a verdict. The navigation timing entry has the status at no
/// protocol-domain cost.
pub(crate) fn refused(document: &Value) -> Option<String> {
    let status = document["status"].as_u64()?;
    (status >= 400).then(|| {
        format!(
            "The server answered {status} for this page: what is shown is its refusal, not the \
             document that was asked for."
        )
    })
}

/// What is over the page and the ways out it offers.
pub(crate) fn overlay(over: &Overlay) -> String {
    let kind = if over.modal { "A dialog" } else { "Something" };
    let what = match over.what.is_empty() {
        true => String::new(),
        false => format!(" — {:?}", over.what),
    };
    let choices = match over.choices.is_empty() {
        true => " It offers no way out that this can see.".to_owned(),
        false => format!(" It offers: {}.", over.choices.join(", ")),
    };
    format!("{kind} is over the page{what}.{choices} Answer it before the page beneath it.")
}

/// Where the canvases are and what to do about them; `told`, once the agent
/// has heard that in full, only where they are and the way to see them.
/// `blank` when this browser has no WebGL to draw them with.
pub(crate) fn canvas(each: &str, coverage: f64, told: bool, blank: bool) -> String {
    let share = (coverage * 100.0).round();
    if told {
        return format!("Canvas ({share}% of the view: {each}): `screenshot` shows it, x,y act on it.");
    }
    let mut note = format!(
        "Canvas ({share}% of the view: {each}): what it draws is in neither the tree nor the text. `screenshot` \
         shows it; click, hover and drag act on it by the x,y of a viewport screenshot; the app's own \
         search, URL parameters, data (eval) or fetched data (`network`) often say more than its pixels."
    );
    if blank {
        note.push_str(
            " This browser has no WebGL, so a map or 3D view may draw blank or fall back: prefer the \
             site's lists and search.",
        );
    }
    note
}

/// The text the canvases newly draw, at most `limit` long.
pub(crate) fn drawn(fresh: &[String], limit: usize) -> Option<String> {
    let fresh: Vec<String> = fresh.iter().map(|item| format!(" {item};")).collect();
    (!fresh.is_empty()).then(|| {
        listed("Text drawn on the canvas, at its viewport x,y:", &fresh, limit.saturating_sub(1), |more| {
            format!(" … and {more} more")
        })
    })
}

/// What became of the wall in front of the page.
pub(crate) fn wall(stand: &Stand) -> String {
    let what = describe(&stand.wall);
    let Some((passed, took, _)) = &stand.stood else {
        return format!("The page shows {what}.");
    };
    let said = match passed {
        Passed::Waited => format!("{} cleared on its own.", sentence(&what)),
        Passed::Ticked => format!("{} cleared after its box was ticked.", sentence(&what)),
        Passed::Puzzle => format!(
            "{} wants a picture puzzle solved. Take a screenshot, click the tiles it asks \
             for by x and y, then its verify button; or ask a person.",
            sentence(&what)
        ),
        Passed::Stuck => format!(
            "Still behind {what}. Wait and try again, retry-clean once, or another route; with a \
             window open a person can tick it. Do not hammer it."
        ),
    };
    format!("{said} Waited {:.1} s at it.", took.as_secs_f64())
}

fn describe(wall: &Wall) -> String {
    match wall {
        Wall::Cloudflare => "a Cloudflare challenge".into(),
        Wall::Box(host) => format!("a {host} checkbox challenge"),
        Wall::DataDome => "a DataDome challenge".into(),
        Wall::PerimeterX => "a PerimeterX press-and-hold challenge".into(),
        Wall::AwsWaf => "an AWS WAF human check; its puzzle is for a person to solve".into(),
        Wall::AwsWafChallenge => "an AWS WAF challenge".into(),
        Wall::Blocked => {
            "a hard block page; do not retry through it, that is what gets an address banned".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_said_on_one_line_whatever_the_page_named_it() {
        let record = serde_json::json!({ "suggestedFilename": "report.csv\nFAKE SUCCESS" });
        let said = event(&Note::Download { completed: true, record });
        assert_eq!(said.lines().count(), 1, "{said}");
        assert!(said.starts_with("Chrome reported download completed: {"), "{said}");
    }
}
