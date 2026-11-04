//! Text on its way to the model: byte-bounded slicing of UTF-8, the fences that
//! quote what a page says, passages around a phrase, and base64.

pub(crate) mod base64;
pub(crate) mod fence;
pub(crate) mod passages;

/// What marks a shortened string.
const ELLIPSIS: char = '…';

/// The longest prefix of `text` that fits in `limit` bytes.
pub(crate) fn head(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    &text[..floor(text, limit)]
}

pub(crate) fn floor(text: &str, at: usize) -> usize {
    (0..=at.min(text.len())).rev().find(|at| text.is_char_boundary(*at)).unwrap_or(0)
}

pub(crate) fn ceil(text: &str, at: usize) -> usize {
    (at.min(text.len())..=text.len()).find(|at| text.is_char_boundary(*at)).unwrap_or(text.len())
}

pub(crate) fn listed(
    head: &str,
    entries: &[String],
    budget: usize,
    more: impl Fn(usize) -> String,
) -> String {
    let mut out = head.to_owned();
    for (shown, entry) in entries.iter().enumerate() {
        if out.len() + entry.len() > budget {
            out.push_str(&more(entries.len() - shown));
            break;
        }
        out.push_str(entry);
    }
    out
}

/// A bounded prefix containing only complete lines. Accessibility rows and
/// their ref identifiers must not become syntactically plausible fragments.
pub(crate) fn head_lines(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let end = text.as_bytes()[..limit].iter().rposition(|byte| *byte == b'\n').map_or(0, |at| at + 1);
    &text[..end]
}

/// Shorten `text` to `limit` bytes, marking that something was dropped.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    format!("{}{ELLIPSIS}", head(text, limit.saturating_sub(ELLIPSIS.len_utf8())))
}

/// Cap output that is about to enter model context, saying so where it stops.
pub(crate) fn bound(value: String, limit: usize) -> String {
    if value.len() <= limit {
        return value;
    }
    format!("{}\n[output truncated; showing first {limit} bytes]", head(&value, limit))
}

/// A value as JSON to read: compact, and a list of objects one to a line.
pub(crate) fn json(value: &serde_json::Value) -> String {
    match value.as_array() {
        Some(items) if !items.is_empty() && items.iter().all(serde_json::Value::is_object) => {
            format!(
                "[\n{}\n]",
                items.iter().map(serde_json::Value::to_string).collect::<Vec<_>>().join(",\n")
            )
        }
        _ => value.to_string(),
    }
}

/// "a thing" at the start of a sentence.
pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_is_compact_and_a_list_of_objects_is_one_to_a_line() {
        let rows = serde_json::json!([{"a": 1}, {"a": 2}]);
        assert_eq!(super::json(&rows), "[\n{\"a\":1},\n{\"a\":2}\n]");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&super::json(&rows)).unwrap(), rows);
        assert_eq!(super::json(&serde_json::json!({"a": [1, 2]})), "{\"a\":[1,2]}");
    }

    #[test]
    fn every_budget_preserves_whole_ref_rows_and_utf8() {
        let rows =
            ["URL: https://example.test/\n", "- button \"Далее\" @e1a.31\n", "- link \"注文\" @e1a.32"];
        let text = rows.concat();
        for budget in 0..=text.len() + 1 {
            let shown = super::head_lines(&text, budget);
            assert!(shown.len() <= budget);
            assert!(text.starts_with(shown));
            assert!(shown.is_empty() || shown == text || shown.ends_with('\n'));
            for line in shown.lines() {
                assert!(
                    rows.iter().any(|row| row.trim_end_matches('\n') == line),
                    "fragment at budget {budget}: {line}"
                );
            }
        }
    }
}
