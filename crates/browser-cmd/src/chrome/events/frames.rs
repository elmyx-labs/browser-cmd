//! Which document generation each frame holds, by observing session.

use std::collections::HashMap;

use serde_json::Value;

/// Which document generation each frame holds. A detached and re-attached
/// frame never gets its old generation back.
#[derive(Default)]
pub(super) struct Frames {
    next: u64,
    /// Observing sessions and their current document generation, by frame.
    pub(super) documents: HashMap<String, HashMap<String, u64>>,
}

impl Frames {
    fn mark(&mut self, session: &str, frame: &str) {
        self.next += 1;
        self.documents.entry(frame.into()).or_default().insert(session.into(), self.next);
    }

    pub(super) fn register(&mut self, session: &str, tree: &Value) {
        visit_frames(tree, &mut |frame| {
            if !self.documents.get(frame).is_some_and(|sessions| sessions.contains_key(session)) {
                self.mark(session, frame);
            }
        });
    }

    pub(super) fn event(&mut self, session: &str, method: &str, params: &Value) {
        match method {
            "Page.frameNavigated" => visit_frames(params, &mut |frame| self.mark(session, frame)),
            "Page.frameAttached" => {
                if let Some(frame) = params["frameId"].as_str() {
                    self.mark(session, frame);
                }
            }
            "Page.frameDetached" => {
                if let Some(frame) = params["frameId"].as_str()
                    && let Some(sessions) = self.documents.get_mut(frame)
                {
                    sessions.remove(session);
                    if sessions.is_empty() {
                        self.documents.remove(frame);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn observers(&self, frame: &str) -> impl Iterator<Item = &String> {
        self.documents.get(frame).into_iter().flat_map(|sessions| sessions.keys())
    }

    pub(super) fn detach(&mut self, session: &str) {
        self.documents.retain(|_, sessions| {
            sessions.remove(session);
            !sessions.is_empty()
        });
    }
}

fn visit_frames(tree: &Value, visit: &mut impl FnMut(&str)) {
    if let Some(frame) = tree["frame"]["id"].as_str() {
        visit(frame);
    }
    for child in tree["childFrames"].as_array().into_iter().flatten() {
        visit_frames(child, visit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_document_generations_ignore_history_and_never_reuse_a_detached_frame_generation() {
        let mut frames = Frames::default();
        frames.register("a", &json!({"frame":{"id":"root"},"childFrames":[{"frame":{"id":"child"}}]}));
        let (root, child) = (frames.documents["root"]["a"], frames.documents["child"]["a"]);
        frames.event("a", "Page.navigatedWithinDocument", &json!({"frameId":"root"}));
        assert_eq!(frames.documents["root"]["a"], root);
        frames.event("a", "Page.frameDetached", &json!({"frameId":"child"}));
        assert!(!frames.documents.contains_key("child"));
        frames.event("a", "Page.frameAttached", &json!({"frameId":"child"}));
        assert_ne!(frames.documents["child"]["a"], child);
        assert_eq!(frames.documents["root"]["a"], root);
        frames.event("a", "Page.frameNavigated", &json!({"frame":{"id":"root"}}));
        assert_ne!(frames.documents["root"]["a"], root);
    }
}
