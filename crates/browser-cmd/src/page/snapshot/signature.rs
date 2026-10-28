//! A fingerprint of the latest render, for telling movement from stillness.

use std::hash::{Hash, Hasher};

use serde_json::Value;

use super::Refs;

impl Refs {
    /// Fingerprint of the latest render's accessibility content at a URL and
    /// root scroll state — the single source of "what state are we in", read
    /// by the awareness log to tell movement from stillness and a fresh state
    /// from a revisited one. `None` when nothing was captured or a frame could
    /// not be read. It does not prove visual stability or a business outcome.
    pub(crate) fn signature(&self, url: &str, viewport: &Value, complete: bool) -> Option<u64> {
        if self.observed.is_empty() || !complete {
            return None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (url, &self.observed, viewport.to_string()).hash(&mut hasher);
        Some(hasher.finish())
    }

    pub(super) fn observe(&mut self, node: &Value, url: &str, role: &str, name: &str) {
        let mut properties: Vec<(&str, String)> = node["properties"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let value = &p["value"]["value"];
                (!value.is_null() && !value.is_array() && !value.is_object())
                    .then(|| (p["name"].as_str().unwrap_or_default(), value.to_string()))
            })
            .collect();
        properties.sort();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (url, role, name, node["value"]["value"].to_string(), properties).hash(&mut hash);
        self.observed.push(hash.finish());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn signature(node: Value, full: bool) -> Option<u64> {
        let mut refs = Refs::default();
        refs.render(&[("main".into(), "page".into(), vec![node])], full, true).unwrap();
        refs.signature("page", &Value::Null, true)
    }

    #[test]
    fn values_numbers_and_control_properties_are_observed_without_dom_identity_noise() {
        let original = json!({"nodeId":"1","backendDOMNodeId":1,"role":{"value":"checkbox"},"name":{"value":"Item 1"},"value":{"value":"a"},"properties":[{"name":"checked","value":{"value":false}}]});
        let base = signature(original.clone(), false);
        let mut value = original.clone();
        value["value"]["value"] = json!("a ");
        assert_ne!(base, signature(value, false));
        let mut name = original.clone();
        name["name"]["value"] = json!("Item 2");
        assert_ne!(base, signature(name, false));
        let mut checked = original.clone();
        checked["properties"][0]["value"]["value"] = json!(true);
        assert_ne!(base, signature(checked, false));
        let mut replaced = original.clone();
        replaced["nodeId"] = json!("200");
        replaced["backendDOMNodeId"] = json!(200);
        assert_eq!(base, signature(replaced, false));
        assert_eq!(base, signature(original, true));
    }

    #[test]
    fn read_only_text_and_root_scroll_are_part_of_the_captured_state() {
        let mut refs = Refs::default();
        let frame = |text: &str| {
            vec![(
                "main".into(),
                "page".into(),
                vec![json!({"nodeId":"1","role":{"value":"StaticText"},"name":{"value":text}})],
            )]
        };
        refs.render(&frame("Price 10"), false, false).unwrap();
        let before = refs.signature("page", &Value::Null, true);
        assert!(before.is_some());
        assert_eq!(refs.signature("page", &Value::Null, false), None, "an unread frame leaves it unknown");
        refs.render(&frame("Price 20"), false, false).unwrap();
        assert_ne!(before, refs.signature("page", &Value::Null, true));
        let before = refs.signature("page", &Value::Null, true);
        assert_ne!(before, refs.signature("page", &json!({"top":100}), true));
        let revision = refs.revision();
        refs.forget();
        refs.render(&frame("Next document"), false, false).unwrap();
        assert_ne!(revision, refs.revision());
    }
}
