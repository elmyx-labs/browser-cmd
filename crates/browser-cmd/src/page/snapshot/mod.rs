//! The page as the model reads it: Chromium's accessibility tree, rendered as
//! indented lines with opaque `@e1a.31` handles on everything that can be acted
//! on. A handle names a DOM node for as long as the document lives, so the
//! button called `@e1a.31` in one state is `@e1a.31` in the next, and an element can
//! also be named by the words on it.

mod ax;
pub(crate) mod capture;
mod delta;
mod fold;
mod handles;
mod line;
mod resolve;
mod signature;

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use ax::{ROLELESS, TEXT, ax_text, has_property, is_interactive};
use delta::{compact, described, rest};
use fold::LANDMARKS;
use line::{Line, tail};

pub(crate) use handles::is_handle;

/// A node a handle names, in the frame it lives in, by its role and name.
#[derive(Clone)]
pub(crate) struct Reference {
    pub(crate) handle: String,
    pub(crate) node: i64,
    pub(crate) frame_id: String,
    pub(crate) role: String,
    pub(crate) name: String,
}

/// How many handles of pages already left are worth remembering. A session
/// that walks a hundred pages should not carry all of them; what is wanted is
/// the page just left, which is where a stale ref almost always comes from.
const RETIRED_KEPT: usize = 512;

/// Handles issued for one document, and what the latest state showed.
#[derive(Default)]
pub(crate) struct Refs {
    by_node: HashMap<i64, String>,
    documents: HashMap<String, Vec<(String, u64)>>,
    /// Every line the previous render of this document showed, so a line
    /// that was not there is marked as new: what a click added, what a scroll
    /// brought in. Empty on a fresh document, where everything is new and
    /// saying so would say nothing.
    before: HashMap<String, usize>,
    shown: HashMap<String, usize>,
    /// The value each handle showed in the previous render, and in this one.
    /// Keyed by handle rather than by role and name, because two controls can
    /// share both of those and a handle names exactly one node.
    was_value: HashMap<String, String>,
    value: HashMap<String, String>,
    /// What the previous render showed and this one does not.
    gone: Vec<String>,
    revision: u64,
    /// This document generation's ref namespace, and the next number in it.
    tag: Option<String>,
    next: u64,
    observed: Vec<u64>,
    /// The latest actionable refs, also the observation's ordered evidence.
    evidence: Vec<Reference>,
    /// Current evidence offsets; unlike `issued` this excludes past renders.
    evidence_by_node: HashMap<i64, usize>,
    /// What the handles of documents now gone used to name. A ref belongs to
    /// one document, and an agent that acts on a page it has left is holding
    /// a name for a thing that may still be on the new page under a new one.
    /// Remembering what the old name meant turns a dead end into the words to
    /// look for.
    retired: HashMap<String, (String, String)>,
    /// What each handle of this document named, for finding the element again
    /// when the page re-rendered it under another node.
    issued: HashMap<String, Reference>,
    /// The lines each site's repeated parts showed, by host, across documents.
    sites: HashMap<String, HashSet<String>>,
    /// This render's repeated parts: their line ranges, role and name.
    landmarks: Vec<(usize, usize, String, String)>,
    in_landmark: bool,
    /// What of the latest render an action needs to be told: the lines it
    /// changed, and how many controls it left as they were.
    compact: Option<(String, usize)>,
}

impl Refs {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn frame_for(&self, node: i64) -> Option<&str> {
        self.evidence.get(*self.evidence_by_node.get(&node)?).map(|reference| reference.frame_id.as_str())
    }

    #[cfg(test)]
    pub(crate) fn references(&self) -> Vec<Reference> {
        self.evidence.clone()
    }

    pub(crate) fn forget(&mut self) {
        // Never give a later document the name of an earlier document's node —
        // but keep what those names meant, bounded, for the message that has
        // to be given when one of them is used again.
        let mut retired = std::mem::take(&mut self.retired);
        if retired.len() > RETIRED_KEPT {
            retired.clear();
        }
        for reference in self.evidence.drain(..).filter(|reference| !reference.name.is_empty()) {
            retired.insert(reference.handle, (reference.role, reference.name));
        }
        let sites = std::mem::take(&mut self.sites);
        *self = Self { revision: self.revision, retired, sites, ..Self::default() };
    }

    /// Render the nodes of `Accessibility.getFullAXTree`, one list per frame:
    /// the page's own first, then each frame under a line that names it.
    pub(crate) fn render(
        &mut self,
        frames: &[(String, String, Vec<Value>)],
        full: bool,
        urls: bool,
    ) -> crate::Result<(String, bool)> {
        self.revision = self.revision.wrapping_add(1);
        self.observed.clear();
        self.evidence.clear();
        self.evidence_by_node.clear();
        self.before = std::mem::take(&mut self.shown);
        self.was_value = std::mem::take(&mut self.value);
        self.gone.clear();
        self.landmarks.clear();
        self.in_landmark = false;
        let mut lines = Vec::new();
        for (index, (frame_id, label, nodes)) in frames.iter().enumerate() {
            let depth = usize::from(index > 0);
            if index > 0 {
                lines.push(Line { head: format!("- frame {label:?}"), titled: true, ..Line::default() });
            }
            let by_id: HashMap<&str, &Value> =
                nodes.iter().filter_map(|node| Some((node["nodeId"].as_str()?, node))).collect();
            let roots: Vec<&Value> = nodes
                .iter()
                .filter(|node| node["parentId"].as_str().is_none_or(|parent| !by_id.contains_key(parent)))
                .collect();
            let frame = Frame { nodes, frame_id, url: label, by_id, full, urls };
            let mut seen = HashSet::new();
            for root in roots {
                self.render_node(root, &frame, depth, "", &mut seen, &mut lines)?;
            }
        }
        // What the previous render showed and this one does not. A line
        // cannot carry this mark the way `[new]` does, because the line is
        // gone; a spinner that finished and a dialog that closed are both
        // only visible as an absence.
        self.gone = self
            .before
            .keys()
            .filter(|key| !self.shown.contains_key(*key))
            .map(|key| described(key))
            .collect();
        self.gone.sort();
        if let Some((_, url, _)) = frames.first() {
            self.fold_repeated(url, &mut lines);
        }
        if lines.is_empty() {
            self.compact = None;
            return Ok(("No interactive elements in this capture. The page may still be rendering or use non-text content. Try wait, snapshot full, text, or screenshot; this is not proof the page is empty.".into(), true));
        }
        let full = lines.iter().map(Line::text).collect::<Vec<_>>().join("\n");
        self.compact = (!self.before.is_empty())
            .then(|| compact(&lines, self.evidence.len()))
            .filter(|(changed, unchanged)| changed.len() + rest(*unchanged, false).len() < full.len());
        Ok((full, false))
    }

    fn render_node<'a>(
        &mut self,
        node: &'a Value,
        frame: &Frame<'a>,
        depth: usize,
        within: &str,
        seen: &mut HashSet<&'a str>,
        lines: &mut Vec<Line>,
    ) -> crate::Result<()> {
        let id = node["nodeId"].as_str().unwrap_or_default();
        if !seen.insert(id) {
            return Ok(());
        }
        let role = ax_text(&node["role"]);
        let name = ax_text(&node["name"]);
        let ignored = node["ignored"].as_bool().unwrap_or(false);
        let interactive = is_interactive(node, &role);
        let meaningful =
            !ignored && !ROLELESS.contains(&role.as_str()) && (!role.is_empty() || !name.is_empty());
        if (meaningful || interactive)
            && role != "RootWebArea"
            && (!name.is_empty() || !node["value"]["value"].is_null() || interactive)
        {
            self.observe(node, frame.url, &role, &name);
        }
        // Headings orient without being actionable: they are what a person
        // reads to know which part of the page the controls belong to. Text
        // that only repeats the name of the thing it is inside — the words of
        // a link, under that link — is not read twice by a reader either, and
        // text with no words at all is the space between two elements.
        let restates = role == TEXT && (name.is_empty() || name == within);
        // A nameless focusable wrapper — a scroll box, a widget's root — gives
        // the agent nothing to choose it by; its contents carry the words.
        let wrapper = matches!(role.as_str(), "generic" | "none") && name.is_empty();
        let shown =
            !restates && !wrapper && (interactive || (meaningful && (frame.full || role == "heading")));
        // A closed list is one control: its options are said on its line.
        let closed = role == "combobox" && !has_property(node, "expanded", "true");
        if shown {
            let value = ax_text(&node["value"]);
            let issued = match node["backendDOMNodeId"].as_i64() {
                Some(backend) if interactive => Some(self.issue(backend, frame.frame_id, &role, &name)?),
                _ => None,
            };
            let (handle, fresh) = issued.map_or((None, false), |(handle, fresh)| (Some(handle), fresh));
            let mark = self.mark(&role, &name, handle.as_deref(), &value, fresh);
            let line =
                Line::node(depth, &role, &name, handle, mark, tail(node, frame, &name, &value, closed));
            if !lines.last_mut().is_some_and(|last| last.join(&line)) {
                lines.push(line);
            }
        }
        if shown && closed {
            return Ok(());
        }
        let child_depth = if shown { depth + 1 } else { depth };
        let within = if shown { name.as_str() } else { within };
        let landmark = !frame.full && !frame.urls && LANDMARKS.contains(&role.as_str()) && !self.in_landmark;
        if landmark {
            // A landmark with a line of its own is named there already.
            let (role, name) =
                if shown { (String::new(), String::new()) } else { (role.clone(), name.clone()) };
            self.landmarks.push((lines.len(), lines.len(), role, name));
            self.in_landmark = true;
        }
        self.render_children(node, frame, id, child_depth, within, seen, lines)?;
        if landmark && let Some(open) = self.landmarks.last_mut() {
            open.1 = lines.len();
            self.in_landmark = false;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn render_children<'a>(
        &mut self,
        node: &'a Value,
        frame: &Frame<'a>,
        id: &str,
        child_depth: usize,
        within: &str,
        seen: &mut HashSet<&'a str>,
        lines: &mut Vec<Line>,
    ) -> crate::Result<()> {
        match node["childIds"].as_array() {
            Some(children) => {
                for child in children.iter().filter_map(Value::as_str) {
                    if let Some(child) = frame.by_id.get(child) {
                        self.render_node(child, frame, child_depth, within, seen, lines)?;
                    }
                }
            }
            // Some Chrome versions omit childIds on the root but still
            // provide parentId. Recover those nodes in document order.
            None => {
                for child in frame.nodes.iter().filter(|candidate| candidate["parentId"].as_str() == Some(id))
                {
                    self.render_node(child, frame, child_depth, within, seen, lines)?;
                }
            }
        }
        Ok(())
    }
}

/// The read-only context of one frame's render, invariant down the recursion.
struct Frame<'a> {
    nodes: &'a [Value],
    frame_id: &'a str,
    url: &'a str,
    by_id: HashMap<&'a str, &'a Value>,
    full: bool,
    /// Whether to spell out where each link goes. The state an action returns
    /// carries no link URLs — the agent acts by ref, and on a page of results
    /// the URLs are most of its weight; an explicit snapshot carries them.
    urls: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn nodes() -> Vec<Value> {
        serde_json::from_value(serde_json::json!([
            {"nodeId":"1","ignored":false,"role":{"type":"role","value":"RootWebArea"},"name":{"type":"computedString","value":"Demo"},"childIds":["2","3","4","5"]},
            {"nodeId":"2","ignored":false,"role":{"type":"role","value":"heading"},"name":{"type":"computedString","value":"Welcome"},"parentId":"1","backendDOMNodeId":2},
            {"nodeId":"3","ignored":false,"role":{"type":"role","value":"button"},"name":{"type":"computedString","value":"Continue"},"parentId":"1","backendDOMNodeId":3},
            {"nodeId":"4","ignored":false,"role":{"type":"role","value":"link"},"name":{"type":"computedString","value":"Docs"},"parentId":"1","backendDOMNodeId":4,
             "properties":[{"name":"url","value":{"type":"string","value":"https://example.com/docs"}}]},
            {"nodeId":"5","ignored":false,"role":{"type":"role","value":"link"},"name":{"type":"computedString","value":"API docs"},"parentId":"1","backendDOMNodeId":5}
        ]))
        .unwrap()
    }

    pub(super) fn page(id: i64) -> Vec<(String, String, Vec<Value>)> {
        vec![(
            "main".into(),
            "page".into(),
            vec![
                json!({"nodeId":"1","backendDOMNodeId":id,"role":{"value":"button"},"name":{"value":"Continue"}}),
            ],
        )]
    }

    pub(super) fn text(id: &str, parent: &str, words: &str) -> Value {
        json!({"nodeId":id,"parentId":parent,"role":{"value":"StaticText"},"name":{"value":words}})
    }

    pub(super) fn rendered(nodes: Vec<Value>) -> String {
        Refs::default().render(&[("main".into(), "page".into(), nodes)], true, false).unwrap().0
    }

    #[test]
    fn snapshot_refs_only_actionable_dom_nodes_and_keeps_them_across_snapshots() {
        let mut refs = Refs::default();
        let page = vec![("main".to_owned(), "page".to_owned(), nodes())];
        let interactive = refs.render(&page, false, true).unwrap().0;
        let button = refs.handle_for(3).unwrap();
        let docs = refs.handle_for(4).unwrap();
        assert!(interactive.contains("heading \"Welcome\""), "headings orient: {interactive}");
        assert!(interactive.contains(&format!("button \"Continue\" @{button}")), "{interactive}");
        assert!(
            interactive.contains(&format!("link \"Docs\" @{docs} [url=https://example.com/docs]")),
            "{interactive}"
        );
        assert_eq!(refs.resolve(&format!("@{button}"), &[]), Ok(3));
        assert_eq!(refs.resolve(&button, &[]), Ok(3), "with or without the sigil");
        let full = refs.render(&page, true, true).unwrap().0;
        assert!(full.contains(&format!("@{button}")), "the same node keeps its name: {full}");
        refs.forget();
        assert!(refs.resolve(&button, &[]).is_err(), "a navigation forgets every handle");
    }

    #[test]
    fn frames_are_rendered_under_a_line_that_names_them() {
        let mut refs = Refs::default();
        let mut inner = nodes();
        inner.truncate(3);
        for node in &mut inner {
            node["backendDOMNodeId"] =
                serde_json::json!(node["backendDOMNodeId"].as_i64().map(|id| id + 100));
        }
        let rendered = refs
            .render(
                &[
                    ("main".to_owned(), "page".to_owned(), nodes()),
                    ("child".to_owned(), "https://pay.example/form".to_owned(), inner),
                ],
                false,
                true,
            )
            .unwrap()
            .0;
        assert!(rendered.contains("- frame \"https://pay.example/form\"\n  - heading"), "{rendered}");
        assert_eq!(
            refs.resolve(&refs.handle_for(103).unwrap(), &[]),
            Ok(103),
            "the frame's button has a handle of its own"
        );
        let references = refs.references();
        assert_eq!(
            references
                .iter()
                .map(|reference| (reference.name.as_str(), reference.frame_id.as_str()))
                .collect::<Vec<_>>(),
            [("Continue", "main"), ("Docs", "main"), ("API docs", "main"), ("Continue", "child"),],
            "the unified index retains both render order and frame ownership"
        );
        for (reference, (handle, node)) in references.iter().zip(refs.handles()) {
            assert_eq!((&reference.handle, reference.node), (&handle, node));
            assert_eq!(refs.frame_for(node), Some(reference.frame_id.as_str()));
        }
    }

    #[test]
    fn old_refs_cannot_alias_a_new_document_or_verify_a_removed_node() {
        let mut refs = Refs::default();
        refs.render(&page(10), false, false).unwrap();
        let original = refs.handle_for(10).unwrap();
        assert_eq!(refs.resolve(&original, &[]), Ok(10));
        assert!(refs.render(&[], false, false).unwrap().1, "nothing captured is said to be empty");
        assert!(refs.resolve(&original, &[]).is_err());
        refs.render(&page(10), false, false).unwrap();
        assert_eq!(refs.resolve(&original, &[]), Ok(10));
        refs.forget();
        refs.render(&page(20), false, false).unwrap();
        assert!(refs.resolve(&original, &[]).is_err());
        assert_eq!(refs.resolve(&refs.handle_for(20).unwrap(), &[]), Ok(20));
    }

    #[test]
    fn frame_lookup_uses_only_current_evidence_even_while_old_nodes_are_remembered() {
        let mut refs = Refs::default();
        refs.render(&page(10), false, false).unwrap();
        assert_eq!(refs.frame_for(10), Some("main"));
        refs.render(&[], false, false).unwrap();
        assert!(refs.handle_for(10).is_some(), "the node remains remembered within its document");
        assert_eq!(refs.frame_for(10), None, "the old index exposed only the current evidence");
    }

    #[test]
    fn text_that_only_restates_what_holds_it_is_not_shown_again() {
        let nodes = vec![
            json!({"nodeId":"1","backendDOMNodeId":1,"role":{"value":"link"},
                   "name":{"value":"pdf"},"childIds":["2","3"]}),
            text("2", "1", "pdf"),
            text("3", "1", "   "),
        ];
        let shown = rendered(nodes);
        assert_eq!(shown.matches("pdf").count(), 1, "{shown}");
        assert_eq!(shown.lines().count(), 1, "{shown}");
    }
}
