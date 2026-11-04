//! Standing at a bot wall the way a person does: waiting, then ticking its
//! box with a real pointer.

use std::time::Duration;

use tokio::time::Instant;

use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::page::settle::{Settled, settle};
use crate::page::walls::{CHALLENGE_FRAMES, CHECKBOX_FRAMES, Wall, detect, frame};
use crate::page::world::World;
use crate::settings::Waits;

/// How standing at a wall went.
pub(crate) enum Passed {
    Waited,
    Ticked,
    /// The box was ticked and a picture puzzle came up instead.
    Puzzle,
    Stuck,
}

/// Where the box sits in a checkbox widget, as fractions of its width and height:
/// the three vendors draw it in the same place.
const CHECKBOX_X: (f64, f64) = (0.05, 0.09);
const CHECKBOX_Y: (f64, f64) = (0.44, 0.56);

/// The wall in front of the page and, when it was one to stand at, how that
/// went, how long it took, and how the page settled after.
pub(crate) struct Stand {
    pub(crate) wall: Wall,
    pub(crate) stood: Option<(Passed, Duration, Settled)>,
}

impl Stand {
    /// The wall still in front of the page.
    pub(crate) fn standing(&self) -> Option<&Wall> {
        match self.stood {
            Some((Passed::Waited | Passed::Ticked, ..)) => None,
            _ => Some(&self.wall),
        }
    }
}

/// Meet whatever stands in front of the page: wait out or tick a passable
/// wall; one that cannot be passed is left standing.
pub(crate) async fn face(world: &mut World, tab: &Tab, waits: &Waits, hand: &mut Hand) -> Option<Stand> {
    let wall = detect(world, tab, waits).await.ok().flatten()?;
    if !wall.passable() {
        return Some(Stand { wall, stood: None });
    }
    let began = Instant::now();
    let mut passed = stand(world, tab, waits, hand, &wall).await;
    let took = began.elapsed();
    let settled = settle(world, tab, waits).await;
    // A challenge that reloads itself changes the page without letting
    // it through: only a wall no longer standing once it settles is gone.
    if matches!(passed, Passed::Waited | Passed::Ticked)
        && !detect(world, tab, waits).await.is_ok_and(|now| now.is_none())
    {
        passed = Passed::Stuck;
    }
    Some(Stand { wall, stood: Some((passed, took, settled)) })
}

/// Stand at a wall the way a person does: read for a moment, since it often
/// lets the page through on its own; then, when it wants a box ticked, tick
/// it with a real pointer, since the movement is what it scores. A hard
/// block is left alone: retrying through one is what gets an address banned.
///
/// The whole stand, waiting and ticking, is held to one `challenge` budget,
/// so a wall cannot take a command past the deadline its caller works to.
async fn stand(world: &mut World, tab: &Tab, waits: &Waits, hand: &mut Hand, wall: &Wall) -> Passed {
    let deadline = Instant::now() + waits.challenge;
    hand.think().await;
    // A token challenge has no box: its whole budget is the wait.
    let patience = match wall {
        Wall::AwsWafChallenge => deadline,
        _ => (Instant::now() + waits.challenge_auto).min(deadline),
    };
    if watch(world, tab, waits, wall, patience, false).await.is_some() {
        return Passed::Waited;
    }
    let Some((_, widget)) = frame(tab, &CHECKBOX_FRAMES).await else {
        return Passed::Stuck;
    };
    // A glance across the widget, then the box.
    let (gx, gy) = (
        widget.left + widget.width * hand.between(0.3, 0.8),
        widget.top + widget.height * hand.between(0.2, 0.8),
    );
    let _ = hand.move_to(tab, gx, gy).await;
    hand.pause(crate::input::hand::tempo::BETWEEN_MOVES).await;
    let (bx, by) = (CHECKBOX_X, CHECKBOX_Y);
    let x = widget.left + widget.width * hand.between(bx.0, bx.1);
    let y = widget.top + widget.height * hand.between(by.0, by.1);
    if hand.click(tab, x, y).await.is_err() {
        return Passed::Stuck;
    }
    // Wait for the box to clear the wall — but give up the moment an image
    // puzzle opens instead, since picking pictures is not something waiting
    // resolves.
    match watch(world, tab, waits, wall, deadline, true).await {
        Some(true) => Passed::Ticked,
        Some(false) => Passed::Puzzle,
        None => Passed::Stuck,
    }
}

async fn watch(
    world: &mut World,
    tab: &Tab,
    waits: &Waits,
    wall: &Wall,
    deadline: Instant,
    puzzle: bool,
) -> Option<bool> {
    loop {
        if detect(world, tab, waits).await.is_ok_and(|now| now.as_ref() != Some(wall)) {
            return Some(true);
        }
        if puzzle && frame(tab, &CHALLENGE_FRAMES).await.is_some() {
            return Some(false);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(waits.challenge_poll).await;
    }
}
