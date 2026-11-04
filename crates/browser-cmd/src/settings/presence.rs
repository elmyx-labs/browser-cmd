//! How the browser presents the agent's pages, as a person's browser would.
//!
//! A person has one page in front: shown and with the focus, the one they
//! act in. Their other tabs hide behind it; their other windows are shown
//! without the focus. A page cannot tell a tab from a window of its own: it
//! sees its visibility and focus, and whether the input it gets fits them.
//!
//! Three answers follow from whether a window is shown and whose the browser is:
//! where a new tab goes ([`Settings::placement`]), whether the tab turned to
//! is brought forward ([`Settings::brings_forward`]) and whether pages behind
//! other windows keep running ([`Settings::runs_behind`]). The focus is not a
//! question: the active tab alone is told it has it, in `tabs::turn_to`.

use crate::Settings;

/// Where a new tab goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In front, in the session's own window: the newest tab is the shown
    /// one and the rest hide behind it, as a person's background tabs do.
    Front,
    /// A window of its own, behind the windows a person is using: shown
    /// without being brought forward. A tab behind another is hidden to its
    /// page, no animation frames and throttled timers, where a tab alone in
    /// its window is not.
    Apart,
}

impl Settings {
    /// In front where nothing is shown and the browser is ours; apart where a
    /// window is shown or the browser may be somebody's.
    pub fn placement(&self) -> Placement {
        match self.headless && self.attach.is_none() {
            true => Placement::Front,
            false => Placement::Apart,
        }
    }

    /// Whether the tab the session turns to is brought to the front, window
    /// and browser with it: free where nothing is shown, a person's wish
    /// (`follow`) where it would take their focus.
    pub fn brings_forward(&self) -> bool {
        self.headless || self.follow
    }

    /// Whether pages behind other windows keep rendering and running their
    /// timers: where a window is shown, since nothing brings them forward.
    pub fn runs_behind(&self) -> bool {
        !self.headless
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_answers_follow_from_a_shown_window_and_whose_browser_it_is() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::new(temp.path(), temp.path());
        settings.headless = true;
        assert_eq!(settings.placement(), Placement::Front);
        assert!(settings.brings_forward() && !settings.runs_behind());
        settings.attach = Some("http://127.0.0.1:9222".into());
        assert_eq!(settings.placement(), Placement::Apart, "somebody's browser: a window of its own");
        settings.attach = None;
        settings.headless = false;
        assert_eq!(settings.placement(), Placement::Apart);
        assert!(!settings.brings_forward() && settings.runs_behind());
        settings.follow = true;
        assert!(settings.brings_forward());
    }
}
