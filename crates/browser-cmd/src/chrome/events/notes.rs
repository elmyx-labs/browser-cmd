//! What tabs volunteered that an action's answer says: per observing session,
//! and held by target for whoever attaches next once a session is let go.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// One action's worth of notes; a page that fails every request must not
/// crowd out the state it failed on.
pub(super) const MAX_NOTES: usize = 20;

/// What a tab volunteered that an action's answer says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Note {
    /// A dialog the page opened, answered at once as the host chose in advance.
    Dialog {
        kind: String,
        message: String,
        accepted: bool,
    },
    /// What Chrome reported of a download that ended, as evidence.
    Download {
        completed: bool,
        record: Value,
    },
    /// A request the page made that failed, of a resource type, and why.
    Failed {
        kind: String,
        reason: String,
    },
    /// A request the page made that the server refused.
    Refused {
        url: String,
        status: u64,
    },
    Crashed,
}

#[derive(Default)]
pub(super) struct Notes {
    tabs: HashMap<String, Vec<Note>>,
    /// Sessions whose failed requests are said, which the operator asks for.
    noted: HashSet<String>,
    /// Notes a tab volunteered and nobody took before its session was let
    /// go, by target, for whoever attaches to it next.
    outbox: HashMap<String, Vec<Note>>,
}

impl Notes {
    pub(super) fn add(&mut self, session: &str, note: Note) {
        if session.is_empty() {
            return;
        }
        let notes = self.tabs.entry(session.into()).or_default();
        if notes.len() < MAX_NOTES {
            notes.push(note);
        }
    }

    pub(super) fn notice_failures(&mut self, session: &str) {
        self.noted.insert(session.into());
    }

    pub(super) fn failures_noticed(&self, session: &str) -> bool {
        self.noted.contains(session)
    }

    /// Those its target held from an earlier attachment first.
    pub(super) fn take(&mut self, session: &str, target: &str) -> Vec<Note> {
        let mut notes = self.outbox.remove(target).unwrap_or_default();
        notes.extend(self.tabs.get_mut(session).map(std::mem::take).unwrap_or_default());
        notes
    }

    pub(super) fn reloaded(&mut self, session: &str, target: &str) {
        let news = |note: &Note| *note != Note::Crashed;
        self.tabs.get_mut(session).into_iter().for_each(|notes| notes.retain(news));
        self.outbox.get_mut(target).into_iter().for_each(|held| held.retain(news));
    }

    /// What a session let go has not said yet waits for its target.
    pub(super) fn release(&mut self, session: &str, target: &str) {
        let notes = self.tabs.get_mut(session).map(std::mem::take).unwrap_or_default();
        self.hold(target, notes);
    }

    pub(super) fn hold(&mut self, target: &str, notes: Vec<Note>) {
        if !notes.is_empty() {
            let outbox = self.outbox.entry(target.into()).or_default();
            outbox.extend(notes);
            outbox.truncate(MAX_NOTES);
        }
    }

    /// A closed page's notes go with it.
    pub(super) fn closed(&mut self, target: &str) {
        self.outbox.remove(target);
    }

    pub(super) fn detach(&mut self, session: &str) {
        self.tabs.remove(session);
        self.noted.remove(session);
    }
}
