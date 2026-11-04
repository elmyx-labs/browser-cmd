//! What the browser volunteers, routed to who keeps it: which frames hold which
//! documents, whether a page is still moving, what a dialog or a download or a
//! failed request said, the tab's own log, its traffic, and the browser's
//! pages. The reader of the connection applies every event here; everything
//! above reads it back.

mod activity;
mod downloads;
mod frames;
mod logs;
mod notes;
pub(crate) mod targets;

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{Value, json};

use crate::settings::Dialogs;
use activity::Activity;
use downloads::{Downloads, Owner, download};
use frames::Frames;
use logs::Logs;
pub(crate) use notes::Note;
use notes::Notes;

#[derive(Default)]
pub(crate) struct Events {
    frames: Frames,
    activity: Activity,
    notes: Notes,
    logs: Logs,
    downloads: Downloads,
    pub(crate) traffic: crate::chrome::traffic::Traffic,
    pub(crate) targets: targets::Targets,
    /// Sessions attached and not yet detached, recorded before the attaching caller wakes.
    pub(crate) attachments: HashSet<String>,
}

impl Events {
    /// Apply one message from the browser. A dialog must be answered at once or
    /// its tab stops answering anything, so the answer to send comes back.
    pub(crate) fn apply(&mut self, message: &Value, dialogs: Dialogs) -> Option<Value> {
        let session = message["sessionId"].as_str();
        let params = &message["params"];
        let method = message["method"].as_str().unwrap_or_default();
        if let Some(session) = session {
            self.frames.event(session, method, params);
            self.activity.event(session, method, params);
            self.traffic.event(session, method, params);
        }
        if session.is_none() {
            self.targets.event(method, params);
        }
        if method == "Target.targetDestroyed"
            && let Some(target) = params["targetId"].as_str()
        {
            self.notes.closed(target);
            self.downloads.close(target);
            self.traffic.asked.remove(target);
        }
        if method == "Target.detachedFromTarget"
            && let Some(session) = params["sessionId"].as_str()
        {
            if let Some(target) = params["targetId"].as_str() {
                self.release(session, target);
            }
            self.detach(session);
        }
        if message.get("id").is_some() {
            return None;
        }
        self.volunteered(session.unwrap_or_default(), method, params, dialogs)
    }

    fn detach(&mut self, session: &str) {
        self.attachments.remove(session);
        self.frames.detach(session);
        self.activity.detach(session);
        self.notes.detach(session);
        self.logs.detach(session);
        self.downloads.detach(session);
        self.traffic.detach(session);
    }

    /// Keep this tab's traffic for `network`, and say its failed requests when `noted`.
    pub(crate) fn watch_network(&mut self, session: &str, noted: bool) {
        self.traffic.record(session);
        if noted {
            self.notes.notice_failures(session);
        }
    }

    /// A message with no id is an event the browser volunteered: a dialog to
    /// answer, a download to report once it lands, a request the page asked
    /// for and did not get, a log line.
    fn volunteered(
        &mut self,
        session: &str,
        method: &str,
        params: &Value,
        dialogs: Dialogs,
    ) -> Option<Value> {
        match method {
            "Page.javascriptDialogOpening" => {
                let kind = params["type"].as_str().unwrap_or("modal");
                let accepted = dialogs.accepts(kind);
                let message = params["message"].as_str().unwrap_or_default().into();
                self.notes.add(session, Note::Dialog { kind: kind.into(), message, accepted });
                return Some(json!({
                    "sessionId": session, "method": "Page.handleJavaScriptDialog",
                    "params": { "accept": accepted, "promptText": params["defaultPrompt"] },
                }));
            }
            // Browser events carry a frame id at begin, then only a guid. Capture
            // the frame's observers at begin; never assign a file to the next caller.
            "Browser.downloadWillBegin" => {
                if let (Some(guid), Some(frame), Some(name)) = (
                    params["guid"].as_str(),
                    params["frameId"].as_str(),
                    params["suggestedFilename"].as_str(),
                ) {
                    let owners: Vec<_> = self.frames.observers(frame).cloned().map(Owner::Session).collect();
                    if !owners.is_empty() {
                        self.downloads.started.insert(guid.into(), (name.into(), owners));
                    }
                }
            }
            "Browser.downloadProgress"
                if matches!(params["state"].as_str(), Some("completed" | "canceled")) =>
            {
                if let Some(guid) = params["guid"].as_str()
                    && let Some((name, owners)) = self.downloads.started.remove(guid)
                {
                    let note = download(name, guid, params);
                    for owner in owners {
                        match owner {
                            Owner::Session(session) => self.notes.add(&session, note.clone()),
                            Owner::Target(target) => self.notes.hold(&target, vec![note.clone()]),
                        }
                    }
                }
            }
            // Reported only when the operator asked for it, because it takes
            // `Network.enable`, and every domain enabled is one more tell.
            "Network.loadingFailed" if self.notes.failures_noticed(session) => self.notes.add(
                session,
                Note::Failed {
                    kind: params["type"].as_str().unwrap_or("resource").to_lowercase(),
                    reason: params["errorText"].as_str().unwrap_or("no reason given").into(),
                },
            ),
            "Network.responseReceived"
                if params["response"]["status"].as_u64().unwrap_or_default() >= 400
                    && self.notes.failures_noticed(session) =>
            {
                let response = &params["response"];
                let url = response["url"].as_str().unwrap_or_default().into();
                self.notes.add(
                    session,
                    Note::Refused { url, status: response["status"].as_u64().unwrap_or_default() },
                )
            }
            "Inspector.targetCrashed" => self.notes.add(session, Note::Crashed),
            "Log.entryAdded" if !session.is_empty() => self.logs.add(session, &params["entry"]),
            _ => {}
        }
        None
    }

    pub(crate) fn network_quiet(&self, session: &str) -> bool {
        self.activity.network_quiet(session)
    }

    /// A counter that moves on every committed navigation and every frame
    /// attached or detached. A page that leaves and returns to the same URL
    /// during a capture still moves it, which an identity comparison cannot see.
    pub(crate) fn navigation_epoch(&self, session: &str) -> u64 {
        self.activity.navigation_epoch(session)
    }

    pub(crate) fn navigation_quiet(&self, session: &str, interval: Duration) -> bool {
        self.activity.navigation_quiet(session, interval)
    }

    /// The notes a tab volunteered, those its target held from an earlier attachment first.
    pub(crate) fn take_notes(&mut self, session: &str, target: &str) -> Vec<Note> {
        self.notes.take(session, target)
    }

    /// A page reloaded since its renderer died: its crash is no longer news.
    pub(crate) fn reloaded(&mut self, session: &str, target: &str) {
        self.notes.reloaded(session, target);
    }

    /// A session let go, by this connection or by the browser: what it has
    /// not said yet, and the downloads it was to be told of, wait for its target.
    pub(crate) fn release(&mut self, session: &str, target: &str) {
        self.attachments.remove(session);
        self.downloads.release(session, target);
        self.notes.release(session, target);
    }

    /// The rolling window of log entries, newest last, left in place so a later
    /// `console` reads the same history.
    pub(crate) fn logs(&self, session: &str) -> Vec<Value> {
        self.logs.read(session)
    }

    /// The generation of the document a frame holds, as last seen.
    pub(crate) fn frame_document(&self, session: &str, frame: &str) -> Option<u64> {
        self.frames.documents.get(frame).and_then(|sessions| sessions.get(session)).copied()
    }

    /// Seed frame ownership from a tree that already exists: an open document
    /// need not emit `frameNavigated` after the Page domain is enabled.
    pub(crate) fn register_frames(&mut self, session: &str, tree: &Value) {
        self.frames.register(session, tree);
    }
}

#[cfg(test)]
mod tests {
    use super::logs::LOG_RING;
    use super::notes::MAX_NOTES;
    use super::*;

    fn apply(events: &mut Events, message: Value) {
        events.apply(&message, Dialogs::Accept);
    }

    #[test]
    fn a_completed_download_belongs_to_its_original_frame_observers() {
        let mut events = Events::default();
        events.register_frames("a", &json!({"frame":{"id":"root"},"childFrames":[{"frame":{"id":"child"}}]}));
        events.register_frames("b", &json!({"frame":{"id":"other"}}));
        apply(
            &mut events,
            json!({"method":"Browser.downloadWillBegin","params":{"guid":"g","frameId":"child","suggestedFilename":"report.csv"}}),
        );
        apply(
            &mut events,
            json!({"sessionId":"a","method":"Page.frameDetached","params":{"frameId":"child"}}),
        );
        events.register_frames("b", &json!({"frame":{"id":"child"}}));
        apply(
            &mut events,
            json!({"method":"Browser.downloadProgress","params":{"guid":"g","state":"completed","receivedBytes":12}}),
        );
        assert!(events.take_notes("b", "B").is_empty(), "another caller must not steal a completion");
        let notes = events.take_notes("a", "A");
        assert!(
            matches!(&notes[..], [Note::Download { record, .. }] if record["suggestedFilename"] == "report.csv")
        );
        assert!(events.take_notes("a", "A").is_empty());
        assert!(events.downloads.started.is_empty());
    }

    #[test]
    fn unknown_downloads_are_not_broadcast_and_detachment_reclaims_tracking() {
        let mut events = Events::default();
        apply(
            &mut events,
            json!({"method":"Browser.downloadWillBegin","params":{"guid":"unknown","frameId":"unknown","suggestedFilename":"private.csv"}}),
        );
        assert!(events.downloads.started.is_empty());
        events.register_frames("a", &json!({"frame":{"id":"root"}}));
        apply(
            &mut events,
            json!({"method":"Browser.downloadWillBegin","params":{"guid":"known","frameId":"root","suggestedFilename":"report.csv"}}),
        );
        apply(&mut events, json!({"method":"Target.detachedFromTarget","params":{"sessionId":"a"}}));
        assert!(events.frames.documents.is_empty() && events.downloads.started.is_empty());
        for guid in ["known", "unknown"] {
            apply(
                &mut events,
                json!({"method":"Browser.downloadProgress","params":{"guid":guid,"state":"completed"}}),
            );
        }
        assert!(events.take_notes("a", "A").is_empty());
        assert!(events.take_notes("b", "B").is_empty());
    }

    #[test]
    fn noisy_tabs_cannot_crowd_out_another_tabs_notes_or_logs() {
        let mut events = Events::default();
        events.watch_network("a", true);
        events.watch_network("b", true);
        apply(
            &mut events,
            json!({"sessionId":"c","method":"Network.loadingFailed","params":{"errorText":"unasked"}}),
        );
        assert!(events.take_notes("c", "C").is_empty(), "failures are said only where asked for");
        for n in 0..MAX_NOTES + LOG_RING {
            apply(
                &mut events,
                json!({"sessionId":"a","method":"Network.loadingFailed","params":{"errorText":format!("a-{n}")}}),
            );
            apply(
                &mut events,
                json!({"sessionId":"a","method":"Log.entryAdded","params":{"entry":{"text":format!("a-{n}")}}}),
            );
        }
        apply(
            &mut events,
            json!({"sessionId":"b","method":"Network.loadingFailed","params":{"errorText":"b-failure"}}),
        );
        apply(
            &mut events,
            json!({"sessionId":"b","method":"Log.entryAdded","params":{"entry":{"text":"b-log"}}}),
        );
        assert_eq!(events.take_notes("b", "B").len(), 1);
        assert_eq!(events.take_notes("a", "A").len(), MAX_NOTES);
        assert_eq!(events.logs("b"), vec![json!({"text":"b-log","level":"info","source":"other","url":""})]);
        assert_eq!(events.logs("a").len(), LOG_RING);
        assert!(events.logs("unknown").is_empty());
    }

    #[test]
    fn notes_not_taken_before_a_session_is_let_go_wait_for_its_target() {
        let mut events = Events::default();
        apply(&mut events, json!({"sessionId":"a","method":"Inspector.targetCrashed","params":{}}));
        events.release("a", "T");
        apply(&mut events, json!({"method":"Target.detachedFromTarget","params":{"sessionId":"a"}}));
        assert!(events.take_notes("b", "U").is_empty(), "another target's are not its");
        assert_eq!(events.take_notes("b", "T"), [Note::Crashed]);
        assert!(events.take_notes("b", "T").is_empty(), "said once");
        apply(&mut events, json!({"sessionId":"c","method":"Inspector.targetCrashed","params":{}}));
        events.release("c", "V");
        events.traffic.asked.insert("V".into());
        apply(&mut events, json!({"method":"Target.targetDestroyed","params":{"targetId":"V"}}));
        assert!(events.take_notes("d", "V").is_empty(), "a closed page's notes go with it");
        assert!(events.traffic.asked.is_empty(), "and the asking to record it");
        apply(&mut events, json!({"sessionId":"e","method":"Inspector.targetCrashed","params":{}}));
        apply(
            &mut events,
            json!({"method":"Target.detachedFromTarget","params":{"sessionId":"e","targetId":"W"}}),
        );
        assert_eq!(events.take_notes("f", "W"), [Note::Crashed], "the browser letting go keeps them too");
    }

    #[test]
    fn a_download_that_ends_after_its_tab_was_let_go_waits_for_that_tab() {
        let mut events = Events::default();
        for (session, guid) in [("a", "kept"), ("b", "closed")] {
            events.register_frames(session, &json!({"frame":{"id":session}}));
            apply(
                &mut events,
                json!({"method":"Browser.downloadWillBegin","params":{"guid":guid,"frameId":session,"suggestedFilename":"r.csv"}}),
            );
        }
        events.release("a", "A");
        apply(
            &mut events,
            json!({"method":"Target.detachedFromTarget","params":{"sessionId":"b","targetId":"B"}}),
        );
        apply(&mut events, json!({"method":"Target.targetDestroyed","params":{"targetId":"B"}}));
        assert_eq!(events.downloads.started.len(), 1, "a closed page's downloads go with it");
        for guid in ["kept", "closed"] {
            apply(
                &mut events,
                json!({"method":"Browser.downloadProgress","params":{"guid":guid,"state":"completed"}}),
            );
        }
        assert!(events.take_notes("a", "B").is_empty());
        assert!(matches!(&events.take_notes("c", "A")[..], [Note::Download { completed: true, .. }]));
    }

    #[test]
    fn a_dialog_is_answered_at_once_and_reported_to_its_tab() {
        let mut events = Events::default();
        let answer = events
            .apply(&json!({"sessionId":"a","method":"Page.javascriptDialogOpening","params":{"type":"confirm","message":"Sure?"}}), Dialogs::AlertsOnly)
            .unwrap();
        assert_eq!(answer["method"], "Page.handleJavaScriptDialog");
        assert_eq!(answer["params"]["accept"], false);
        assert!(matches!(&events.take_notes("a", "A")[..], [Note::Dialog { accepted: false, .. }]));
    }

    #[test]
    fn requested_navigation_and_active_loading_are_distinct_and_frame_scoped() {
        let mut events = Events::default();
        apply(
            &mut events,
            json!({"sessionId":"s","method":"Page.frameRequestedNavigation","params":{"frameId":"main","disposition":"currentTab"}}),
        );
        apply(
            &mut events,
            json!({"sessionId":"s","method":"Page.frameStoppedLoading","params":{"frameId":"child"}}),
        );
        assert!(!events.navigation_quiet("s", Duration::from_secs(1)));
        apply(
            &mut events,
            json!({"sessionId":"s","method":"Page.frameStartedLoading","params":{"frameId":"main"}}),
        );
        assert!(!events.navigation_quiet("s", Duration::ZERO));
        apply(
            &mut events,
            json!({"sessionId":"s","method":"Page.frameStoppedLoading","params":{"frameId":"main"}}),
        );
        assert!(events.navigation_quiet("s", Duration::ZERO));
    }

    #[test]
    fn the_navigation_epoch_moves_on_round_trips_and_is_session_scoped() {
        let mut events = Events::default();
        for url in ["https://example.org/other", "https://example.org/start"] {
            apply(
                &mut events,
                json!({"sessionId":"a","method":"Page.frameNavigated","params":{"frame":{"url":url}}}),
            );
        }
        apply(&mut events, json!({"sessionId":"b","method":"Page.frameDetached","params":{}}));
        apply(&mut events, json!({"sessionId":"a","method":"Log.entryAdded","params":{"entry":{}}}));
        assert_eq!(events.navigation_epoch("a"), 2);
        assert_eq!(events.navigation_epoch("b"), 1);
    }
}
