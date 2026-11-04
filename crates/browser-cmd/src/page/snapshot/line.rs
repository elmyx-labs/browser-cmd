//! How one line of the tree reads, and when two runs of text read as one.

use serde_json::Value;

use super::Frame;
use super::ax::{TEXT, ax_text, options};

/// How much of what a line carries is shown. A name is what the agent acts by,
/// so it gets room; a value is read, and a long one is a document in a textarea.
const NAME_CHARS: usize = 400;
const VALUE_CHARS: usize = 200;

/// How many options of a closed list its line names.
const OPTIONS_SHOWN: usize = 8;

#[derive(Default)]
pub(super) struct Line {
    pub(super) depth: usize,
    pub(super) head: String,
    pub(super) handle: Option<String>,
    pub(super) mark: Option<&'static str>,
    pub(super) tail: String,
    pub(super) words: Option<String>,
    pub(super) titled: bool,
}

impl Line {
    pub(super) fn node(
        depth: usize,
        role: &str,
        name: &str,
        handle: Option<String>,
        mark: Option<&'static str>,
        tail: String,
    ) -> Self {
        let name = crate::text::clip(name, NAME_CHARS);
        let mut head = format!("- {}", if role.is_empty() { "element" } else { role });
        if !name.is_empty() {
            head.push(' ');
            head.push_str(&serde_json::to_string(&name).unwrap_or_default());
        }
        let titled = matches!(role, "heading" | "frame")
            && (!name.is_empty() || handle.is_some() || mark.is_some() || !tail.is_empty());
        let plain = role == TEXT && !name.is_empty() && handle.is_none() && tail.is_empty();
        Self { depth: depth.min(20), head, handle, mark, tail, words: plain.then_some(name), titled }
    }

    pub(super) fn bare(&self) -> String {
        let handle = self.handle.as_ref().map(|handle| format!(" @{handle}")).unwrap_or_default();
        format!("{}{handle}{}{}", self.head, self.mark.unwrap_or_default(), self.tail)
    }

    pub(super) fn text(&self) -> String {
        format!("{}{}", "  ".repeat(self.depth), self.bare())
    }

    /// A line as a site's other pages would show it: no ref, no marks, no indent.
    pub(super) fn key(&self) -> String {
        format!("{}{}", self.head, self.tail)
    }

    /// Two text lines as one, when that is what the page shows: both are plain
    /// runs of words at the same depth, neither carrying a mark of its own.
    /// `false` leaves them apart.
    pub(super) fn join(&mut self, next: &Line) -> bool {
        let (Some(before), Some(after)) = (&self.words, &next.words) else { return false };
        if self.depth != next.depth || before.chars().count() + after.chars().count() >= NAME_CHARS {
            return false;
        }
        let space = !before.ends_with(char::is_whitespace) && !after.starts_with(char::is_whitespace);
        let words = format!("{before}{}{after}", if space { " " } else { "" });
        self.head = format!("- {TEXT} {}", serde_json::to_string(&words).unwrap_or_default());
        // A run that has just appeared is marked, and the mark belongs to the
        // whole of it: the page put those words up together.
        self.mark = self.mark.or(next.mark);
        self.words = Some(words);
        true
    }
}

pub(super) fn tail(node: &Value, frame: &Frame, name: &str, value: &str, closed: bool) -> String {
    let mut line = String::new();
    if !value.is_empty() && value != name {
        line.push_str(&format!(
            " [value={}]",
            serde_json::to_string(&crate::text::clip(value, VALUE_CHARS)).unwrap_or_default()
        ));
    }
    for property in node["properties"].as_array().into_iter().flatten() {
        let key = property["name"].as_str().unwrap_or_default();
        // Where a link goes decides whether to follow it — but only an
        // explicit snapshot spells it out; the state an action returns
        // leaves it off, since the agent follows a link by its ref.
        let keep = matches!(
            key,
            "checked" | "selected" | "expanded" | "disabled" | "required" | "readonly" | "level"
        ) || (key == "url" && frame.urls);
        if keep {
            let value = ax_text(&property["value"]);
            let value = match key {
                "url" => link(&value, frame.url),
                _ => value,
            };
            if !value.is_empty() && value != "false" {
                line.push_str(&format!(" [{key}={}]", crate::text::clip(&value, VALUE_CHARS)));
            }
        }
    }
    if closed {
        let options = options(node, frame);
        if !options.is_empty() {
            let named = options.iter().take(OPTIONS_SHOWN).map(|o| format!("{o:?}")).collect::<Vec<_>>();
            let more = options.len().saturating_sub(OPTIONS_SHOWN);
            line.push_str(&format!(
                " [options: {}{}]",
                named.join(", "),
                match more {
                    0 => String::new(),
                    more => format!(", +{more} more"),
                }
            ));
        }
    }
    line
}

/// A link's address as a line hints at it: on the page's own site its path and
/// query; elsewhere its host and path, with … for a query that `text links=true`
/// spells out.
fn link(link: &str, page: &str) -> String {
    let link = crate::web::shown(link);
    match (url::Url::parse(&link), url::Url::parse(page)) {
        (Ok(link), Ok(page)) if link.origin() == page.origin() => {
            link[url::Position::BeforePath..].to_owned()
        }
        (Ok(link), _) if link.query().is_some() => format!("{}…", &link[..url::Position::AfterPath]),
        _ => link,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{rendered, text};
    use serde_json::json;

    /// A quotation the record keeps in pieces cannot be found.
    #[test]
    fn a_title_that_markup_split_is_shown_as_the_one_line_the_page_shows() {
        let nodes = vec![
            json!({"nodeId":"1","role":{"value":"paragraph"},"name":{"value":"result"},
                   "childIds":["2","3","4"]}),
            text("2", "1", "Computational"),
            text("3", "1", "framework for"),
            text("4", "1", "quantum state tomography"),
        ];
        assert!(
            rendered(nodes).contains("Computational framework for quantum state tomography"),
            "the words the page shows side by side are shown side by side"
        );
    }

    #[test]
    fn a_link_elsewhere_shows_its_host_and_path_and_tracking_is_gone() {
        assert_eq!(super::link("https://a.test/p?x=1&utm_source=y", "https://a.test/"), "/p?x=1");
        assert_eq!(super::link("https://b.test/p?x=1", "https://a.test/"), "https://b.test/p…");
        assert_eq!(super::link("https://b.test/p#top", "https://a.test/"), "https://b.test/p#top");
    }

    #[test]
    fn text_at_different_depths_stays_apart() {
        let nodes = vec![
            json!({"nodeId":"1","role":{"value":"list"},"name":{"value":"l"},"childIds":["2","4"]}),
            json!({"nodeId":"2","parentId":"1","role":{"value":"listitem"},"name":{"value":"i"},
                   "childIds":["3"]}),
            text("3", "2", "first"),
            text("4", "1", "second"),
        ];
        assert!(!rendered(nodes).contains("first second"));
    }
}
