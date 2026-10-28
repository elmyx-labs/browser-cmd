//! What Chromium's accessibility nodes say: their roles, words, properties and options.

use serde_json::Value;

use super::Frame;

/// The ARIA roles an agent can act on. A node with any other role is offered
/// only when the page itself says it takes focus or a value.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "option",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "treeitem",
];

/// The role Chrome gives a run of the page's own words.
pub(super) const TEXT: &str = "StaticText";

/// The roles that carry meaning without being one of the page's own elements.
/// `InlineTextBox` is here because it is not a second thing: Chrome splits a
/// run of text into the boxes layout drew it in, each repeating its parent's
/// words verbatim. Showing both says everything twice. Emphasis and its
/// relatives are here because they are how a word looks, not what it is: they
/// carry no name of their own, and standing between a sentence and the rest
/// of it they turn one line of reading into three.
pub(super) const ROLELESS: &[&str] =
    &["none", "generic", "InlineTextBox", "emphasis", "strong", "subscript", "superscript"];

pub(super) fn has_property(node: &Value, key: &str, value: &str) -> bool {
    node["properties"]
        .as_array()
        .is_some_and(|all| all.iter().any(|p| p["name"] == key && ax_text(&p["value"]) == value))
}

pub(super) fn options(node: &Value, frame: &Frame) -> Vec<String> {
    let mut found = Vec::new();
    for child in node["childIds"].as_array().into_iter().flatten() {
        if let Some(child) = child.as_str().and_then(|id| frame.by_id.get(id)) {
            match ax_text(&child["role"]).as_str() {
                "option" => found.extend(Some(ax_text(&child["name"])).filter(|name| !name.is_empty())),
                _ => found.extend(options(child, frame)),
            }
        }
    }
    found
}

pub(super) fn is_interactive(node: &Value, role: &str) -> bool {
    // The document is focusable and is not a control.
    if node["ignored"].as_bool().unwrap_or(false)
        || node["backendDOMNodeId"].is_null()
        || role == "RootWebArea"
    {
        return false;
    }
    if INTERACTIVE_ROLES.contains(&role) {
        return true;
    }
    node["properties"].as_array().is_some_and(|properties| {
        properties.iter().any(|property| {
            matches!(property["name"].as_str(), Some("focusable" | "settable"))
                && ax_text(&property["value"]) == "true"
        })
    })
}

pub(super) fn ax_text(value: &Value) -> String {
    match &value["value"] {
        Value::String(value) => value.trim().to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::rendered;
    use serde_json::json;

    #[test]
    fn the_boxes_layout_drew_a_run_of_text_in_are_not_a_second_reading_of_it() {
        let nodes = vec![
            json!({"nodeId":"1","role":{"value":"paragraph"},"name":{"value":"p"},"childIds":["2"]}),
            json!({"nodeId":"2","parentId":"1","role":{"value":"StaticText"},
                   "name":{"value":"Once only"},"childIds":["3"]}),
            json!({"nodeId":"3","parentId":"2","role":{"value":"InlineTextBox"},
                   "name":{"value":"Once only"}}),
        ];
        assert_eq!(rendered(nodes).matches("Once only").count(), 1);
    }
}
