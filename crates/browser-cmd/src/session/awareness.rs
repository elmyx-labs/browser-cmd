//! Factual trajectory hints over fresh accessibility observations. No recovery
//! policy, number stripping or inference of visual/business outcomes lives here.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

/// How many steps are remembered, how often one state may recur before it is
/// called a cycle, and how many steps with nothing new make a plateau.
const HISTORY: usize = 16;
const REPEAT: usize = 3;
const PLATEAU: usize = 4;

struct Step {
    action: u64,
    ok: bool,
    state: Option<u64>,
    novel: bool,
}

pub(crate) struct Awareness {
    enabled: bool,
    log: VecDeque<Step>,
    /// Whether a cycle was already explained; after that it is only counted.
    told_cycle: bool,
}

impl Awareness {
    pub(crate) fn new(enabled: bool) -> Self {
        Self { enabled, log: VecDeque::new(), told_cycle: false }
    }

    pub(crate) fn observe(&mut self, action: impl Hash, ok: bool, state: Option<u64>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let mut request = std::collections::hash_map::DefaultHasher::new();
        action.hash(&mut request);
        let action = request.finish();
        let novel = state.is_some_and(|hash| {
            !self.log.iter().rev().take(HISTORY.saturating_sub(1)).any(|step| step.state == Some(hash))
        });
        self.log.push_back(Step { action, ok, state, novel });
        while self.log.len() > HISTORY {
            self.log.pop_front();
        }
        let previous = self.log.iter().rev().nth(1);
        if previous.is_some_and(|old| old.action == action && !old.ok && !ok) {
            return Some(
                "You repeated the same action and it failed again. Inspect the reported error before \
                 retrying; the page's resulting state is not established by that failure."
                    .into(),
            );
        }
        let hash = state?;
        let visits = self.log.iter().filter(|step| step.state == Some(hash)).count();
        if visits >= REPEAT {
            return Some(match std::mem::replace(&mut self.told_cycle, true) {
                true => format!(
                    "The captured state has repeated {visits} times, a possible cycle: verify the effect \
                     before repeating."
                ),
                false => format!(
                    "The captured accessibility state has repeated {visits} times; this may be a cycle in \
                     the observed state. Visual changes and business outcomes may not be represented; \
                     verify the actual effect before repeating actions."
                ),
            });
        }
        let plateau = self.log.len() >= PLATEAU
            && self.log.iter().rev().take(PLATEAU).all(|step| step.state.is_some() && !step.novel);
        if plateau {
            return Some(format!(
                "The last {} actions produced no new captured accessibility state. This does not \
                 establish whether a visual or business effect occurred.",
                PLATEAU
            ));
        }
        if previous.is_some_and(|old| old.action == action && old.state == state) {
            return Some(
                "You repeated the same action and its captured accessibility state repeated. This does \
                 not establish whether a visual or business effect occurred."
                    .into(),
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn on() -> Awareness {
        Awareness::new(true)
    }

    #[test]
    fn changing_content_is_novel_even_when_no_new_dom_nodes_are_created() {
        let mut a = on();
        for hash in 0..10 {
            assert!(a.observe("type", true, Some(hash)).is_none());
        }
    }

    #[test]
    fn unknown_and_canvas_states_never_claim_no_movement() {
        let mut a = on();
        for _ in 0..6 {
            assert!(a.observe("drag", true, None).is_none());
        }
    }

    #[test]
    fn repeated_failure_is_reported_without_claiming_a_page_state() {
        let mut a = on();
        assert!(a.observe("click", false, None).is_none());
        let note = a.observe("click", false, None).unwrap();
        assert!(note.contains("failed again"));
        assert!(!note.contains("did not change"));
    }

    #[test]
    fn cycles_are_scoped_to_captured_state() {
        let mut a = on();
        for (i, hash) in [1, 2, 1, 2].into_iter().enumerate() {
            a.observe(format!("action{i}"), true, Some(hash));
        }
        let note = a.observe("last", true, Some(1)).unwrap();
        assert!(note.contains("cycle") && note.contains("may not be represented"));
        let again = a.observe("again", true, Some(1)).unwrap();
        assert!(again.starts_with("The captured state has repeated 4 times, a possible cycle"), "{again}");
    }

    #[test]
    fn a_disabled_log_says_nothing() {
        let mut a = Awareness::new(false);
        for _ in 0..6 {
            assert!(a.observe("click", false, None).is_none());
        }
    }
}
