//! The browser's own log, a rolling window per observing session, for `console`.

use std::collections::{HashMap, VecDeque};

use serde_json::{Value, json};

/// How much of the browser's own log one tab keeps for `console` to read back.
pub(super) const LOG_RING: usize = 50;

/// How much of one log line is kept.
const LOG_CHARS: usize = 300;

#[derive(Default)]
pub(super) struct Logs {
    tabs: HashMap<String, VecDeque<Value>>,
}

impl Logs {
    /// One `Log.entryAdded` entry: errors, failed requests, CSP — from the `Log`
    /// domain, which does not carry the `Runtime.enable` tell.
    pub(super) fn add(&mut self, session: &str, entry: &Value) {
        let logs = self.tabs.entry(session.into()).or_default();
        if logs.len() >= LOG_RING {
            logs.pop_front();
        }
        logs.push_back(json!({
            "level": entry["level"].as_str().unwrap_or("info"),
            "source": entry["source"].as_str().unwrap_or("other"),
            "text": crate::text::clip(entry["text"].as_str().unwrap_or_default(), LOG_CHARS),
            "url": entry["url"].as_str().unwrap_or_default(),
        }));
    }

    pub(super) fn read(&self, session: &str) -> Vec<Value> {
        self.tabs.get(session).into_iter().flatten().cloned().collect()
    }

    pub(super) fn detach(&mut self, session: &str) {
        self.tabs.remove(session);
    }
}
