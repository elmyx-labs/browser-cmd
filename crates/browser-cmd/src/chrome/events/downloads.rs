//! Downloads begun in observed frames, and what Chrome reported of them.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::Note;

/// Downloads begun in observed frames, indexed by Chrome's unique guid: the
/// suggested name and who is told when each ends.
#[derive(Default)]
pub(super) struct Downloads {
    pub(super) started: HashMap<String, (String, Vec<Owner>)>,
}

/// Who a download's end is said to: a session observing its frame, or, once
/// that session was let go, the page it was in.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Owner {
    Session(String),
    Target(String),
}

impl Downloads {
    /// A session gone with no page to wait for: its downloads are no longer anybody's.
    pub(super) fn detach(&mut self, session: &str) {
        self.keep(|owner| *owner != Owner::Session(session.into()));
    }

    /// A session let go: its downloads wait for its page.
    pub(super) fn release(&mut self, session: &str, target: &str) {
        for (_, owners) in self.started.values_mut() {
            for owner in owners.iter_mut().filter(|owner| **owner == Owner::Session(session.into())) {
                *owner = Owner::Target(target.into());
            }
        }
    }

    /// A page closed: what waited for it goes with it.
    pub(super) fn close(&mut self, target: &str) {
        self.keep(|owner| *owner != Owner::Target(target.into()));
    }

    fn keep(&mut self, keep: impl Fn(&Owner) -> bool) {
        self.started.retain(|_, (_, owners)| {
            owners.retain(&keep);
            !owners.is_empty()
        });
    }
}

/// Browser-reported download evidence, without a local path fabricated from
/// the suggested name: CDP permits a missing filePath even on completion, and
/// a reported path does not prove the file exists on this host.
pub(super) fn download(name: String, guid: &str, params: &Value) -> Note {
    let completed = params["state"] == "completed";
    let path = completed.then(|| params["filePath"].as_str().filter(|path| !path.is_empty())).flatten();
    let bytes = params.get("receivedBytes").filter(|value| value.as_f64().is_some_and(|n| n >= 0.));
    let record =
        json!({ "guid": guid, "suggestedFilename": name, "receivedBytes": bytes, "browserPath": path });
    Note::Download { completed, record }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(note: Note) -> Value {
        let Note::Download { record, .. } = note else { panic!("not a download: {note:?}") };
        record
    }

    #[test]
    fn download_evidence_preserves_reported_path() {
        let record = record(download(
            "report.csv\nFAKE SUCCESS".into(),
            "id",
            &json!({ "state":"completed", "receivedBytes":12.5, "filePath":"/actual/report (2).csv" }),
        ));
        assert_eq!(record["suggestedFilename"], "report.csv\nFAKE SUCCESS");
        assert_eq!(record["browserPath"], "/actual/report (2).csv");
        assert_eq!(record["receivedBytes"], 12.5);
    }

    #[test]
    fn absent_download_evidence_is_not_guessed_and_cancellation_has_no_completed_path() {
        for params in [
            json!({"state":"completed"}),
            json!({"state":"completed","receivedBytes":-1,"filePath":""}),
            json!({"state":"completed","receivedBytes":"12","filePath":12}),
            json!({"state":"canceled","filePath":"/unfinished/file"}),
        ] {
            let record = record(download("suggestion.csv".into(), "id", &params));
            assert!(record["browserPath"].is_null(), "{record}");
            assert!(record["receivedBytes"].is_null(), "{record}");
            assert_eq!(record["guid"], "id");
        }
    }
}
