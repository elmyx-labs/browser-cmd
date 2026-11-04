//! The parts every page of a site repeats, folded once a page of that site has shown them.

use super::Refs;
use super::line::Line;

/// The parts of a page every page of a site repeats: shown once per site, then
/// folded to one line while their controls stay addressable by name.
pub(super) const LANDMARKS: &[&str] = &["banner", "navigation", "contentinfo"];

/// How many lines of repeated parts a session remembers, per site and in sites.
const SITE_LINES: usize = 4000;
const SITES: usize = 64;

impl Refs {
    /// On a new document, fold each repeated part — a site's banner, menus and
    /// footer — that an earlier page of the same site already showed whole.
    /// Its controls keep their refs and are acted on by name; `snapshot`
    /// prints them all.
    pub(super) fn fold_repeated(&mut self, url: &str, lines: &mut Vec<Line>) {
        let landmarks = std::mem::take(&mut self.landmarks);
        let Some(host) = url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_owned)) else {
            return;
        };
        if self.sites.len() >= SITES && !self.sites.contains_key(&host) {
            self.sites.clear();
        }
        let known = self.sites.entry(host).or_default();
        let fresh = self.before.is_empty();
        for (start, end, role, name) in landmarks.into_iter().rev() {
            let end = end.min(lines.len());
            if end <= start {
                continue;
            }
            let keys: Vec<String> = lines[start..end].iter().map(Line::key).collect();
            if fresh && end - start > 1 && keys.iter().all(|key| known.contains(key)) {
                let what = match (role.as_str(), name.as_str()) {
                    ("", _) => String::new(),
                    (role, "") => format!("{role}: "),
                    (role, name) => format!("{role} {name:?}: "),
                };
                let head = format!(
                    "- {what}{} lines as on an earlier page of this site; act on them by name",
                    end - start
                );
                let folded = Line { depth: lines[start].depth, head, ..Line::default() };
                lines.splice(start..end, [folded]);
            } else if known.len() + keys.len() <= SITE_LINES {
                known.extend(keys);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Refs;
    use serde_json::Value;

    #[test]
    fn repeated_parts_fold_lists_close_and_links_shorten() {
        let page = |url: &str, button: i64| {
            vec![(
                "main".to_owned(),
                url.to_owned(),
                serde_json::from_value::<Vec<Value>>(serde_json::json!([
                    {"nodeId":"1","role":{"value":"RootWebArea"},"childIds":["2","5","6"]},
                    {"nodeId":"2","role":{"value":"navigation"},"name":{"value":"Main"},"parentId":"1","childIds":["3","4"]},
                    {"nodeId":"3","role":{"value":"link"},"name":{"value":"Home"},"parentId":"2","backendDOMNodeId":3,
                     "properties":[{"name":"url","value":{"value":"https://shop.test/home?x=1"}}]},
                    {"nodeId":"4","role":{"value":"link"},"name":{"value":"Deals"},"parentId":"2","backendDOMNodeId":4},
                    {"nodeId":"5","role":{"value":"button"},"name":{"value":"Buy"},"parentId":"1","backendDOMNodeId":button},
                    {"nodeId":"6","role":{"value":"combobox"},"name":{"value":"Size"},"parentId":"1","backendDOMNodeId":8,
                     "childIds":["7"],"properties":[{"name":"expanded","value":{"value":false}}]},
                    {"nodeId":"7","role":{"value":"MenuListPopup"},"parentId":"6","childIds":["9","10"]},
                    {"nodeId":"9","role":{"value":"option"},"name":{"value":"S"},"parentId":"7","backendDOMNodeId":9},
                    {"nodeId":"10","role":{"value":"option"},"name":{"value":"M"},"parentId":"7","backendDOMNodeId":10}
                ]))
                .unwrap(),
            )]
        };
        let mut refs = Refs::default();
        let first = refs.render(&page("https://shop.test/a", 5), false, false).unwrap().0;
        assert!(first.contains("link \"Home\"") && first.contains("link \"Deals\""), "{first}");
        assert!(first.contains("combobox \"Size\"") && first.contains("[options: \"S\", \"M\"]"), "{first}");
        assert!(!first.contains("option \"S\""), "{first}");
        refs.forget();
        let second = refs.render(&page("https://shop.test/b", 11), false, false).unwrap().0;
        assert!(second.contains("- navigation \"Main\": 2 lines as on an earlier page"), "{second}");
        assert!(!second.contains("link \"Home\""), "{second}");
        assert_eq!(refs.resolve("Deals", &[]), Ok(4), "folded controls are acted on by name");
        refs.forget();
        let other = refs.render(&page("https://elsewhere.test/", 12), false, false).unwrap().0;
        assert!(other.contains("link \"Home\""), "another site is shown whole: {other}");
        refs.forget();
        let listed = refs.render(&page("https://shop.test/c", 13), false, true).unwrap().0;
        assert!(listed.contains("link \"Home\"") && listed.contains("[url=/home?x=1]"), "{listed}");
    }
}
