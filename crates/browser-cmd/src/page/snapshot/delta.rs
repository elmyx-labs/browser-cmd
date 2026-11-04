//! What changed since the last render: what is new, what went away, and the compact account of it.

use super::Refs;
use super::line::Line;

/// How many departed elements one state names before it counts the rest.
const GONE_SHOWN: usize = 12;

/// The mark a line carries when the page has only just put it up.
pub(super) const NEW: &str = " [new]";

impl Refs {
    pub(super) fn mark(
        &mut self,
        role: &str,
        name: &str,
        handle: Option<&str>,
        value: &str,
        fresh: bool,
    ) -> Option<&'static str> {
        if let Some(handle) = handle {
            self.value.insert(handle.to_owned(), value.to_owned());
        }
        let key = format!("{role}\u{1}{name}");
        let count = self.shown.entry(key.clone()).or_default();
        *count += 1;
        // A ref the agent has not seen is new to it, even where a control
        // of that name was shown before: a re-rendered or repeated one.
        if !self.before.is_empty() && (fresh || *count > self.before.get(&key).copied().unwrap_or(0)) {
            Some(NEW)
        } else if let Some(handle) = handle
            && self.was_value.get(handle).is_some_and(|was| *was != value)
        {
            // The same node, showing something else: what typing and
            // choosing do, which neither appearing nor going away records.
            Some(" [changed]")
        } else {
            None
        }
    }

    /// What the latest render changed, when it had a baseline in this
    /// document and its changes say less than the whole tree would; `told`,
    /// once the agent knows what the count of the rest means, says it shorter.
    pub(crate) fn compact(&self, told: bool) -> Option<String> {
        let (changed, unchanged) = self.compact.as_ref()?;
        let rest = rest(*unchanged, told);
        Some(if changed.is_empty() { rest } else { format!("{changed}\n{rest}") })
    }

    /// What the previous render of this document showed and this one does
    /// not, as one line, capped. `None` when nothing went away, and on a
    /// fresh document, where there is no previous render to have shown it.
    pub(crate) fn gone(&self) -> Option<String> {
        if self.gone.is_empty() {
            return None;
        }
        let shown = self.gone.iter().take(GONE_SHOWN).cloned().collect::<Vec<_>>().join(", ");
        Some(match self.gone.len().saturating_sub(GONE_SHOWN) {
            0 => format!("Gone: {shown}"),
            more => format!("Gone: {shown}, and {more} more"),
        })
    }
}

/// The line that counts the controls a compact state left out.
pub(super) fn rest(unchanged: usize, told: bool) -> String {
    match told {
        true => format!("{unchanged} unchanged — `snapshot` reprints"),
        false => format!("{unchanged} unchanged controls not repeated — `snapshot` reprints"),
    }
}

/// What an action changed on a document already shown: each marked line under
/// the nearest heading or frame above it, and how many controls were left out.
pub(super) fn compact(lines: &[Line], controls: usize) -> (String, usize) {
    let mut out = Vec::new();
    let (mut heading, mut said) = (None, None);
    let mut repeated = 0;
    for line in lines {
        let bare = line.bare();
        if line.titled {
            heading = Some(bare.clone());
        }
        if line.mark.is_none() {
            continue;
        }
        repeated += usize::from(line.handle.is_some());
        if line.titled {
            said.clone_from(&heading);
            out.push(bare);
            continue;
        }
        if heading != said {
            said.clone_from(&heading);
            out.extend(heading.clone());
        }
        out.push(format!("{}{bare}", if said.is_some() { "  " } else { "" }));
    }
    (out.join("\n"), controls.saturating_sub(repeated))
}

/// A `role`-`name` key as it is spoken of once its line is gone.
pub(super) fn described(key: &str) -> String {
    match key.split_once('\u{1}') {
        Some((role, name)) if !name.is_empty() => format!("{role} {name:?}"),
        Some((role, _)) => role.to_owned(),
        None => key.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Refs;
    use super::super::tests::nodes;
    use serde_json::Value;

    fn form(value: &str, with_cancel: bool) -> Vec<Value> {
        let children: Vec<&str> = if with_cancel { vec!["2", "3"] } else { vec!["2"] };
        let mut nodes = serde_json::json!([
            {"nodeId":"1","ignored":false,"role":{"type":"role","value":"RootWebArea"},"name":{"type":"computedString","value":"Cart"},"childIds":children},
            {"nodeId":"2","ignored":false,"role":{"type":"role","value":"textbox"},"name":{"type":"computedString","value":"Promo code"},"parentId":"1","backendDOMNodeId":2,"value":{"type":"string","value":value}}
        ]);
        if with_cancel {
            nodes.as_array_mut().unwrap().push(serde_json::json!(
                {"nodeId":"3","ignored":false,"role":{"type":"role","value":"button"},"name":{"type":"computedString","value":"Cancel"},"parentId":"1","backendDOMNodeId":3}
            ));
        }
        serde_json::from_value(nodes).unwrap()
    }

    /// An action shows itself three ways: what appeared, what went away, and
    /// what now reads differently. The first render of a document has nothing
    /// to compare against and marks none of them.
    #[test]
    fn what_went_away_is_named_and_a_value_that_changed_is_marked() {
        let mut refs = Refs::default();
        let page = |nodes| vec![("main".to_owned(), "page".to_owned(), nodes)];
        let before = refs.render(&page(form("", true)), false, false).unwrap().0;
        assert!(!before.contains("[changed]"), "nothing to compare against yet: {before}");
        assert!(!before.contains("[new]"), "{before}");
        assert_eq!(refs.gone(), None, "a fresh document has shown nothing to lose");

        let after = refs.render(&page(form("SAVE10", false)), false, false).unwrap().0;
        assert!(after.contains("textbox \"Promo code\""), "{after}");
        assert!(after.contains("[changed]"), "the same node reads differently now: {after}");
        assert!(after.contains("[value=\"SAVE10\"]"), "{after}");
        assert!(!after.contains("[new]"), "the field was already there: {after}");
        assert_eq!(refs.gone(), Some("Gone: button \"Cancel\"".to_owned()));

        // Standing still marks nothing at all.
        let again = refs.render(&page(form("SAVE10", false)), false, false).unwrap().0;
        assert!(!again.contains("[changed]") && !again.contains("[new]"), "{again}");
        assert_eq!(refs.gone(), None);

        // A navigation forgets the comparison along with the handles.
        refs.forget();
        let fresh = refs.render(&page(form("", true)), false, false).unwrap().0;
        assert!(!fresh.contains("[new]") && !fresh.contains("[changed]"), "{fresh}");
        assert_eq!(refs.gone(), None);
    }

    /// After an action on the same document, only what it changed is said,
    /// under the heading it sits beneath; the rest is counted, not repeated.
    #[test]
    fn an_action_on_a_shown_document_is_told_only_what_changed() {
        let mut refs = Refs::default();
        let page = |extra: bool| {
            let mut nodes = nodes();
            if extra {
                nodes[0]["childIds"].as_array_mut().unwrap().push(serde_json::json!("6"));
                nodes.push(
                    serde_json::json!({"nodeId":"6","ignored":false,"role":{"type":"role","value":"button"},
                    "name":{"type":"computedString","value":"Undo"},"parentId":"1","backendDOMNodeId":6}),
                );
            }
            vec![("main".to_owned(), "page".to_owned(), nodes)]
        };
        refs.render(&page(false), false, false).unwrap();
        assert!(refs.compact(false).is_none(), "no baseline: the whole tree");
        let (full, empty) = refs.render(&page(true), false, false).unwrap();
        let compact = refs.compact(false).unwrap();
        assert!(!empty && full.contains("Undo") && compact.len() < full.len());
        let undo = refs.handle_for(6).unwrap();
        assert_eq!(
            compact,
            format!(
                "- heading \"Welcome\"\n  - button \"Undo\" @{undo} [new]\n\
                 3 unchanged controls not repeated — `snapshot` reprints"
            )
        );
        refs.render(&page(true), false, false).unwrap();
        assert_eq!(refs.compact(false).unwrap(), "4 unchanged controls not repeated — `snapshot` reprints");
        assert_eq!(
            refs.compact(true).unwrap(),
            "4 unchanged — `snapshot` reprints",
            "said shorter once told"
        );
        // A control the page re-rendered under the same name has a new ref,
        // so it is said again; the ref the agent held still finds it.
        let held = refs.handle_for(3).unwrap();
        let mut rerendered = page(true);
        rerendered[0].2[2]["backendDOMNodeId"] = serde_json::json!(7);
        refs.render(&rerendered, false, false).unwrap();
        let again = refs.handle_for(7).unwrap();
        assert!(refs.compact(false).unwrap().contains(&format!("button \"Continue\" @{again} [new]")));
        let refused = refs.resolve(&held, &[]).unwrap_err();
        assert!(refused.contains(&format!("it is now @{again}")), "named, never rebound: {refused}");
        refs.forget();
        refs.render(&page(true), false, false).unwrap();
        assert!(refs.compact(false).is_none(), "a new document: the whole tree");
        // A delta longer than the tree it describes is not shorter reading.
        let mut lone = Refs::default();
        let one = |name: &str| {
            vec![(
                "main".to_owned(),
                "page".to_owned(),
                vec![serde_json::json!(
                {"nodeId":"1","backendDOMNodeId":1,"role":{"value":"button"},"name":{"value":name}})],
            )]
        };
        lone.render(&one("A"), false, false).unwrap();
        lone.render(&one("B"), false, false).unwrap();
        assert!(lone.compact(false).is_none());
    }
}
