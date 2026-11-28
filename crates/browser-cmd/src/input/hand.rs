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
//! rather than synthetic DOM events. This does not make them identical to
//! physical device input.
//!
//! Every pause is a range of a person's, scaled by one `pace`: a person's
//! tempo at 1, none at 0. Below 1 the hand is faster than a person's, and a
//! page that times it can tell.

use std::time::Duration;

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::input::keys::{SHIFT, dispatch_keystroke, key_event, key_for_char};
use crate::input::rect::Rect;
use crate::{Error, Result};

/// Bounded pause ranges in seconds, sampled with a truncated log-normal
/// distribution and multiplied by the configured pace.
pub(crate) mod tempo {
    pub(crate) const BEFORE_PRESS: (f64, f64) = (0.1, 0.5);
    pub(crate) const HOLD: (f64, f64) = (0.04, 0.15);
    pub(crate) const KEY_HOLD: (f64, f64) = (0.07, 0.17);
    pub(crate) const BETWEEN_NOTCHES: (f64, f64) = (0.012, 0.045);
    /// From letting the button go to pressing it again, in a double click.
    pub(crate) const BETWEEN_CLICKS: (f64, f64) = (0.06, 0.16);
    /// From the mouse to the keys, after the click that put the caret in a field.
    pub(crate) const TO_THE_KEYS: (f64, f64) = (0.3, 0.9);
    /// From letting one key go to pressing the next: with the hold, a quick
    /// typist's pace from key to key.
    pub(crate) const BETWEEN_KEYS: (f64, f64) = (0.02, 0.13);
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
    /// One notch of a wheel, in the pixels Chrome scrolls for it on this
    /// system, and how many events a long scroll is spread over: past that a
    /// faster spin sends several notches in each.
    pub(super) const NOTCH: f64 = if cfg!(target_os = "macos") { 40.0 } else { 100.0 };
    pub(super) const TURNS: f64 = 30.0;
    /// How far a wandering pointer stays from the edges of the view.
    pub(super) const MARGIN: (f64, f64) = (50.0, 80.0);
    pub(super) const WANDERS: (f64, f64) = (1.0, 3.99);
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
        let mut hand = Self {
            x: 0.0,
            y: 0.0,
            seed: now.as_nanos() as u64 ^ u64::from(std::process::id()) | 1,
            held: false,
            pace,
        };
        // Wherever the pointer was left, which is not the same place twice.
        (hand.x, hand.y) = (hand.between(80.0, 1100.0).round(), hand.between(80.0, 640.0).round());
        hand
    }

    /// Whether the pointer drifts about the page before this click, as a
    /// person's hand sometimes does; never at pace 0.
    pub(crate) fn may_wander(&mut self) -> bool {
        self.pace > 0.0 && self.random() < tempo::WANDER
    }

    /// Uniform in `0..1`. xorshift is plenty: nothing here is secret, it only
    /// has to not be the same every time.
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
        let seconds = self.pause_sample(low, high) * self.pace;
        if seconds > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        }
    }

    /// A bounded log-normal draw keeps waits positive and finite. The bounds
    /// remain operational limits; rejection avoids a point mass at either end.
    fn pause_sample(&mut self, low: f64, high: f64) -> f64 {
        if high <= 0.0 {
            return 0.0;
        }
        let low = low.max(f64::EPSILON).min(high);
        if low == high {
            return low;
        }
        let median = (low * high).sqrt();
        let sigma = (high / low).ln() / 4.0;
        loop {
            let u = self.random().max(f64::MIN_POSITIVE);
            let v = self.random();
            let normal = (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos();
            let sample = median * (sigma * normal).exp();
            if (low..=high).contains(&sample) {
                return sample;
            }
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
        let mut event = json!({ "type": kind, "x": on_grid(x), "y": on_grid(y) });
        // While the button is held, every move is part of the drag, and the
        // page is told so on each one; a move without the button is a hover.
        let dragging = self.held && kind != "mouseReleased";
        let mut buttons = i64::from(dragging);
        if let Some((button, count)) = press {
            event["button"] = button.into();
            event["clickCount"] = count.into();
            if kind == "mousePressed" {
                buttons = if button == "right" { 2 } else { 1 };
            }
        }
        if dragging {
            event["button"] = "left".into();
        }
        // What a pressed mouse button reads as on the page: the button held,
        // at the half pressure a device without a pressure sensor reports.
        event["buttons"] = buttons.into();
        event["force"] = (if buttons == 0 { 0.0 } else { 0.5 }).into();
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
            let t = minimum_jerk(step as f64 / steps as f64);
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
            if count > 1 {
                self.pause(tempo::BETWEEN_CLICKS).await;
            }
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
        // Two draws averaged: most presses land near the middle, few near the bounds.
        let mut within = |(low, high): (f64, f64)| (self.between(low, high) + self.between(low, high)) / 2.0;
        (rect.left + rect.width * within(shape::ACROSS), rect.top + rect.height * within(shape::DOWN))
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
        let distance = dx.abs().max(dy.abs());
        let stride = shape::NOTCH * (distance / (shape::NOTCH * shape::TURNS)).ceil().max(1.0);
        let mut left = distance;
        while left > 0.0 {
            let share = stride.min(left) / distance;
            let event = json!({
                "type": "mouseWheel", "x": on_grid(self.x), "y": on_grid(self.y),
                "deltaX": (dx * share).round(), "deltaY": (dy * share).round(),
            });
            tab.call("Input.dispatchMouseEvent", event).await?;
            left -= stride;
            self.pause(tempo::BETWEEN_NOTCHES).await;
        }
        Ok(())
    }

    /// How long a key stays down.
    pub(crate) async fn hold_key(&mut self) {
        self.pause(tempo::KEY_HOLD).await;
    }

    /// Type as a hand does: a key at a time, with the gaps between, and now
    /// and then a hesitation mid-word. Anything a keyboard has a key for goes
    /// through the keyboard; the rest enters the way an input method delivers
    /// it.
    pub(crate) async fn type_text(&mut self, tab: &Tab, text: &str) -> Result<()> {
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            match character {
                '\n' | '\r' => crate::input::keys::press(tab, self, "Enter").await?,
                '\t' => crate::input::keys::press(tab, self, "Tab").await?,
                _ => match key_for_char(character) {
                    Some((key, code, virtual_key, shift)) => {
                        let modifiers = if shift { SHIFT } else { 0 };
                        let text = character.to_string();
                        let mut down = key_event("keyDown", modifiers, &key, &code, virtual_key);
                        down["text"] = Value::String(text.clone());
                        down["unmodifiedText"] = Value::String(text);
                        dispatch_keystroke(tab, self, down).await?;
                    }
                    None => Self::insert(tab, &character.to_string()).await?,
                },
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

/// A coordinate as a pointing device reports it. A Mac's pointer moves in
/// 128ths of a point, which a page reads off its pointer events; elsewhere a
/// mouse reports whole pixels, and a fraction is a computed point.
fn on_grid(coordinate: f64) -> f64 {
    let steps = if cfg!(target_os = "macos") { 128.0 } else { 1.0 };
    (coordinate * steps).round() / steps
}

/// Smooth progress along the curve: zero endpoint velocity and acceleration.
fn minimum_jerk(t: f64) -> f64 {
    t * t * t * (10.0 + t * (-15.0 + 6.0 * t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_progress_is_monotonic_and_slow_at_both_ends() {
        let samples: Vec<f64> = (0..=100).map(|i| minimum_jerk(i as f64 / 100.0)).collect();
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[100], 1.0);
        assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(samples[1] < samples[50] - samples[49]);
        assert!(1.0 - samples[99] < samples[50] - samples[49]);
    }

    #[test]
    fn pause_samples_stay_inside_positive_operational_bounds() {
        let mut hand = Hand::new(1.0);
        hand.seed = 1;
        let samples: Vec<f64> = (0..1000).map(|_| hand.pause_sample(0.05, 0.22)).collect();
        assert!(samples.iter().all(|s| s.is_finite() && (0.05..=0.22).contains(s)));
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!(mean < (0.05 + 0.22) / 2.0, "right skew differs from a uniform draw: {mean}");
        assert_eq!(hand.pause_sample(0.0, 0.0), 0.0);
        assert_eq!(hand.pause_sample(0.1, 0.1), 0.1);
    }

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
    async fn a_key_hold_uses_the_configured_range() {
        let mut hand = Hand::new(1.0);
        let before = tokio::time::Instant::now();
        hand.hold_key().await;
        let held = before.elapsed().as_secs_f64();
        // The paused clock wakes on whole milliseconds.
        assert!((0.07..=0.1715).contains(&held), "{held}");
    }
}
