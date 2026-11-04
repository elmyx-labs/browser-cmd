//! A hand on the pointer and the keys, as a person uses them.
//!
//! A bot wall scores behaviour as well as fingerprints: a pointer that
//! teleports to the centre of a button and presses within the same
//! millisecond is a script, and Turnstile in particular watches the movement
//! that precedes its checkbox being ticked. So the pointer here has a
//! position, travels along a curve with a little noise, lands somewhere in a
//! box rather than at its centre, pauses before it presses, sometimes wanders
//! first, and keys go down and up with the gaps a hand leaves between them.
//! The events are Chrome's own trusted input events, dispatched through CDP,
//! which is what makes them indistinguishable from a real device.
//!
//! Every pause is a range of a person's, scaled by one `pace`: a person's
//! tempo at 1, none at 0.

use std::time::Duration;

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::input::keys::{SHIFT, key_event, key_for_char};
use crate::input::rect::Rect;
use crate::{Error, Result};

/// The pauses of a person, in seconds, each a range drawn from uniformly and
/// multiplied by the pace.
pub(crate) mod tempo {
    pub(crate) const BEFORE_PRESS: (f64, f64) = (0.1, 0.5);
    pub(crate) const HOLD: (f64, f64) = (0.04, 0.15);
    pub(crate) const BETWEEN_KEYS: (f64, f64) = (0.05, 0.22);
    /// How likely a pause mid-word is, and how long it lasts.
    pub(crate) const HESITATION: f64 = 0.05;
    pub(crate) const HESITATE_FOR: (f64, f64) = (0.3, 1.0);
    pub(crate) const READ_SHORT: (f64, f64) = (0.5, 1.6);
    pub(crate) const READ_LONG: (f64, f64) = (1.5, 4.0);
    pub(crate) const READ_LONG_CHANCE: f64 = 0.35;
    pub(crate) const BETWEEN_MOVES: (f64, f64) = (0.2, 0.7);
    pub(crate) const PER_STEP: (f64, f64) = (0.004, 0.014);
    /// How likely an aimless look around the page is before a click.
    pub(crate) const WANDER: f64 = 0.3;
}

/// The shape of a hand's movement, as distinct from its tempo, which is a
/// setting. These are what make a path look drawn rather than computed, and
/// they are facts about the curve rather than knobs a deployment turns.
mod shape {
    /// One step of travel per this many pixels, within these bounds: a short
    /// hop still takes several, a long sweep does not take hundreds.
    pub(super) const PIXELS_PER_STEP: f64 = 25.0;
    pub(super) const STEPS: (f64, f64) = (6.0, 28.0);
    /// How far the path bows off the straight line, as a share of its length.
    pub(super) const BEND: f64 = 0.25;
    /// The tremor left on every step but the last, which lands exactly.
    pub(super) const JITTER: f64 = 1.0;
    /// Where in a box a finger lands: inside it, and not at its centre.
    pub(super) const ACROSS: (f64, f64) = (0.2, 0.8);
    pub(super) const DOWN: (f64, f64) = (0.25, 0.75);
    /// A wheel turns in notches of this many pixels, this many at a time.
    pub(super) const NOTCH: f64 = 120.0;
    pub(super) const NOTCHES: (f64, f64) = (1.0, 8.0);
    /// How far a wandering pointer stays from the edges of the view.
    pub(super) const MARGIN: (f64, f64) = (50.0, 80.0);
    pub(super) const WANDERS: (f64, f64) = (1.0, 3.99);
    /// A keystroke is held a fraction of an ordinary press.
    pub(super) const TYPING_HOLD: f64 = 4.0;
}

/// The pointer and the keys, a little randomness to move with, and the tempo to go at.
pub(crate) struct Hand {
    x: f64,
    y: f64,
    seed: u64,
    /// Whether the left button is down, which makes every move a drag.
    held: bool,
    /// Multiplier on every pause: a person at 1, no pauses at 0.
    pub(crate) pace: f64,
}

impl Hand {
    pub(crate) fn new(pace: f64) -> Self {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        Self {
            x: 200.0,
            y: 300.0,
            seed: now.as_nanos() as u64 ^ u64::from(std::process::id()) | 1,
            held: false,
            pace,
        }
    }

    /// Uniform in `0..1`. xorshift is plenty: nothing here is secret, it only
    /// has to not be the same every time.
    /// Whether the pointer drifts about the page before this click, as a
    /// person's hand sometimes does; never at pace 0.
    pub(crate) fn may_wander(&mut self) -> bool {
        self.pace > 0.0 && self.random() < tempo::WANDER
    }

    pub(crate) fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1_u64 << 53) as f64
    }

    pub(crate) fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.random()
    }

    /// Sleep for a while inside a range of seconds, at this tempo.
    pub(crate) async fn pause(&mut self, (low, high): (f64, f64)) {
        let seconds = self.between(low, high) * self.pace;
        if seconds > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        }
    }

    /// The pause of someone reading before they act: usually short, sometimes
    /// a real look at the page.
    pub(crate) async fn think(&mut self) {
        let long = self.random() < tempo::READ_LONG_CHANCE;
        self.pause(if long { tempo::READ_LONG } else { tempo::READ_SHORT }).await;
    }

    async fn event(
        &self,
        tab: &Tab,
        kind: &str,
        x: f64,
        y: f64,
        press: Option<(&str, i64)>,
        acknowledge: bool,
    ) -> Result<()> {
        let mut event = json!({ "type": kind, "x": x, "y": y });
        if let Some((button, count)) = press {
            event["button"] = button.into();
            event["clickCount"] = count.into();
            // A right press says it holds the button, as a page's context menu reads it.
            if button == "right" {
                event["buttons"] = (if kind == "mousePressed" { 2 } else { 0 }).into();
            }
        }
        // While the button is held, every move is part of the drag, and the
        // page is told so on each one; a move without the button is a hover.
        if self.held {
            event["button"] = "left".into();
            event["buttons"] = 1.into();
        }
        if acknowledge {
            tab.call("Input.dispatchMouseEvent", event).await?;
        } else {
            tab.send("Input.dispatchMouseEvent", event)?;
        }
        Ok(())
    }

    /// Travel to a point along a bent path, in as many steps as the distance
    /// deserves, with the jitter of a hand.
    pub(crate) async fn move_to(&mut self, tab: &Tab, x: f64, y: f64) -> Result<()> {
        let (x0, y0) = (self.x, self.y);
        let distance = ((x - x0).powi(2) + (y - y0).powi(2)).sqrt();
        let steps = (distance / shape::PIXELS_PER_STEP).clamp(shape::STEPS.0, shape::STEPS.1) as usize;
        let bend = self.between(-shape::BEND, shape::BEND) * distance;
        let (cx, cy) = (
            (x0 + x) / 2.0 - (y - y0) / distance.max(1.0) * bend,
            (y0 + y) / 2.0 + (x - x0) / distance.max(1.0) * bend,
        );
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let (jx, jy) = if step < steps {
                (self.between(-shape::JITTER, shape::JITTER), self.between(-shape::JITTER, shape::JITTER))
            } else {
                (0.0, 0.0)
            };
            let px = (1.0 - t).powi(2) * x0 + 2.0 * (1.0 - t) * t * cx + t * t * x + jx;
            let py = (1.0 - t).powi(2) * y0 + 2.0 * (1.0 - t) * t * cy + t * t * y + jy;
            self.event(tab, "mouseMoved", px, py, None, step == steps).await?;
            self.pause(tempo::PER_STEP).await;
        }
        self.x = x;
        self.y = y;
        Ok(())
    }

    pub(crate) async fn click(&mut self, tab: &Tab, x: f64, y: f64) -> Result<()> {
        self.move_to(tab, x, y).await?;
        self.press(tab, x, y, 1).await
    }

    /// A right-button click, which opens a page's own context menu — the
    /// `contextmenu` event, not the OS menu.
    pub(crate) async fn secondary(&mut self, tab: &Tab, x: f64, y: f64) -> Result<()> {
        self.pause(tempo::BEFORE_PRESS).await;
        self.event(tab, "mousePressed", x, y, Some(("right", 1)), true).await?;
        self.pause(tempo::HOLD).await;
        self.event(tab, "mouseReleased", x, y, Some(("right", 1)), true).await
    }

    /// Press and release at a point, `times` clicks in a row — the second of a
    /// double click being a press that counts itself as the second. Split from
    /// the travel so a caller can look at what is under the cursor once it has
    /// arrived, before it commits to the press.
    pub(crate) async fn press(&mut self, tab: &Tab, x: f64, y: f64, times: i64) -> Result<()> {
        self.pause(tempo::BEFORE_PRESS).await;
        for count in 1..=times {
            self.event(tab, "mousePressed", x, y, Some(("left", count)), true).await?;
            self.pause(tempo::HOLD).await;
            self.event(tab, "mouseReleased", x, y, Some(("left", count)), true).await?;
        }
        Ok(())
    }

    /// Press at one point, travel to another with the button down, let go:
    /// a slider dragged, a card moved between columns, a map panned. The
    /// travel is the same bent, jittered path as any other, which is what a
    /// site that scores pointer behaviour expects of a hand.
    pub(crate) async fn drag(&mut self, tab: &Tab, from: (f64, f64), to: (f64, f64)) -> Result<()> {
        self.move_to(tab, from.0, from.1).await?;
        self.pause(tempo::BEFORE_PRESS).await;
        self.event(tab, "mousePressed", from.0, from.1, Some(("left", 1)), true).await?;
        self.held = true;
        let travel = async {
            self.pause(tempo::HOLD).await;
            self.move_to(tab, to.0, to.1).await?;
            self.pause(tempo::HOLD).await;
            Ok::<(), Error>(())
        }
        .await;
        self.held = false;
        travel?;
        self.event(tab, "mouseReleased", to.0, to.1, Some(("left", 1)), true).await
    }

    /// Somewhere inside a box, the way a finger lands: not the centre, not
    /// the edge.
    pub(crate) fn point_in(&mut self, rect: Rect) -> (f64, f64) {
        (
            rect.left + rect.width * self.between(shape::ACROSS.0, shape::ACROSS.1),
            rect.top + rect.height * self.between(shape::DOWN.0, shape::DOWN.1),
        )
    }

    /// Aimless pointer movement, like someone drifting the cursor before they
    /// act. It does not scroll: wander runs right before a click aims at a
    /// target, and a wheel turn here leaves the page mid-scroll — animated, with
    /// momentum — so the box the aim just measured has moved by the time the
    /// press lands, and the click misses (or the cover guard fires by surprise).
    pub(crate) async fn wander(&mut self, tab: &Tab, width: f64, height: f64) -> Result<()> {
        let (across, down) = shape::MARGIN;
        for _ in 0..self.between(shape::WANDERS.0, shape::WANDERS.1) as usize {
            let x = self.between(across, (width - across).max(across + 10.0));
            let y = self.between(down, (height - down).max(down + 10.0));
            self.move_to(tab, x, y).await?;
            self.pause(tempo::BETWEEN_MOVES).await;
        }
        Ok(())
    }

    /// Scroll from where the pointer is, in the notches a wheel turns in.
    pub(crate) async fn wheel(&mut self, tab: &Tab, dx: f64, dy: f64) -> Result<()> {
        let notches =
            ((dx.abs() + dy.abs()) / shape::NOTCH).clamp(shape::NOTCHES.0, shape::NOTCHES.1) as usize;
        for _ in 0..notches {
            let event = json!({
                "type": "mouseWheel", "x": self.x, "y": self.y,
                "deltaX": dx / notches as f64, "deltaY": dy / notches as f64,
            });
            tab.call("Input.dispatchMouseEvent", event).await?;
            self.pause(tempo::HOLD).await;
        }
        Ok(())
    }

    /// How long a key stays down.
    pub(crate) async fn hold_key(&mut self) {
        let (low, high) = tempo::HOLD;
        self.pause((low / shape::TYPING_HOLD, high / shape::TYPING_HOLD)).await;
    }

    /// Type as a hand does: a key at a time, with the gaps between, and now
    /// and then a hesitation mid-word. Anything a keyboard has a key for goes
    /// through the keyboard; the rest enters the way an input method delivers
    /// it.
    pub(crate) async fn type_text(&mut self, tab: &Tab, text: &str) -> Result<()> {
        for character in text.chars() {
            match key_for_char(character) {
                Some((key, code, virtual_key, shift)) => {
                    let modifiers = if shift { SHIFT } else { 0 };
                    let text = character.to_string();
                    let mut down = key_event("keyDown", modifiers, &key, &code, virtual_key);
                    down["text"] = Value::String(text.clone());
                    down["unmodifiedText"] = Value::String(text);
                    tab.call("Input.dispatchKeyEvent", down).await?;
                    self.hold_key().await;
                    tab.call(
                        "Input.dispatchKeyEvent",
                        key_event("keyUp", modifiers, &key, &code, virtual_key),
                    )
                    .await?;
                }
                None => Self::insert(tab, &character.to_string()).await?,
            }
            self.pause(tempo::BETWEEN_KEYS).await;
            if self.random() < tempo::HESITATION {
                self.pause(tempo::HESITATE_FOR).await;
            }
        }
        Ok(())
    }

    /// Text as an input method delivers it: in one piece, with no keys.
    pub(crate) async fn insert(tab: &Tab, text: &str) -> Result<()> {
        tab.call("Input.insertText", json!({ "text": text })).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_never_repeats_itself_and_lands_inside_the_box() {
        let mut hand = Hand::new(1.0);
        let first: Vec<f64> = (0..8).map(|_| hand.random()).collect();
        assert!(first.iter().all(|value| (0.0..1.0).contains(value)));
        assert!(first.windows(2).any(|pair| pair[0] != pair[1]));
        let rect = Rect { left: 100.0, top: 50.0, width: 40.0, height: 20.0 };
        for _ in 0..50 {
            let (x, y) = hand.point_in(rect);
            assert!((108.0..=132.0).contains(&x) && (55.0..=65.0).contains(&y), "{x},{y}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_pace_of_zero_never_sleeps() {
        let mut hand = Hand::new(0.0);
        let before = tokio::time::Instant::now();
        hand.pause((1.0, 2.0)).await;
        hand.think().await;
        hand.hold_key().await;
        assert_eq!(tokio::time::Instant::now(), before);
    }

    #[tokio::test(start_paused = true)]
    async fn a_key_is_held_a_fraction_of_a_press() {
        let mut hand = Hand::new(1.0);
        let before = tokio::time::Instant::now();
        hand.hold_key().await;
        let held = before.elapsed().as_secs_f64();
        // The paused clock wakes on whole milliseconds.
        assert!((0.01..=0.0385).contains(&held), "{held}");
    }
}
