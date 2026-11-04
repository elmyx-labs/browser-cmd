//! Finding the node a target names: a handle, or the words on an element.

use super::handles::is_handle;
use super::{Reference, Refs};

/// How many elements an ambiguous name lists to choose between, and how many
/// near misses are offered when nothing matched at all.
const AMBIGUOUS_SHOWN: usize = 8;
const NEAREST_SHOWN: usize = 6;

/// A word no longer than this is in half the names on the page and names
/// nothing, so a near miss is not matched on it.
const NEAR_WORD_CHARS: usize = 2;

impl Refs {
    /// The node a target names: a handle, or the visible name of an element
    /// in the latest state, whole or in part, when that names one element.
    /// Where a name fits several, the roles the action makes sense for
    /// decide: "Search" typed into is the box, not the button beside it.
    pub(crate) fn resolve(&self, target: &str, roles: &[&str]) -> Result<i64, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("element ref or name must not be empty".into());
        }
        let handle = target.strip_prefix('@').unwrap_or(target);
        if let Some(issued) = self.issued.get(handle) {
            if !self.evidence.iter().any(|reference| reference.handle == handle) {
                return Err(match self.renamed(target) {
                    Some(now) => format!("{target} was re-rendered by the page; it is now {now}"),
                    None => format!("{target} is no longer in the current page state; take a fresh snapshot"),
                });
            }
            return Ok(issued.node);
        }
        if is_handle(target) {
            // The ref is from a page that is gone. What it named there is the
            // one thing that can still be looked for here.
            let was = self.retired.get(handle).map(|(role, name)| format!("{role} {name:?}"));
            let here = self
                .retired
                .get(handle)
                .filter(|(_, name)| !is_handle(name))
                .and_then(|(_, name)| self.resolve(name, roles).ok())
                .and_then(|node| self.by_node.get(&node));
            return Err(match (was, here) {
                (Some(was), Some(now)) => format!(
                    "{target} named the {was} on a page that is gone; in this one that name is @{now}"
                ),
                (Some(was), None) => format!(
                    "{target} named the {was} on a page that is gone, and nothing in this page's state \
                     carries that name; take a snapshot"
                ),
                _ => format!("{target} is not a ref in this document; use one from the latest state"),
            });
        }
        let wanted = target.to_lowercase();
        let named = |exact: bool| -> Vec<&Reference> {
            self.evidence
                .iter()
                .filter(|reference| {
                    let name = reference.name.to_lowercase();
                    if exact { name == wanted } else { name.contains(&wanted) }
                })
                .collect()
        };
        let (exact, partial) = (named(true), named(false));
        let one = |found: Vec<&Reference>| (found.len() == 1).then(|| found[0].node);
        if let Some(node) = one(fitting(&exact, roles))
            .or_else(|| one(exact.clone()))
            .or_else(|| one(fitting(&partial, roles)))
            .or_else(|| one(partial.clone()))
        {
            return Ok(node);
        }
        let many = if exact.is_empty() { partial } else { exact };
        match many.as_slice() {
            [] => {
                // Offer the nearest actionable elements — those whose name shares
                // a word with the ask — so a near-miss self-corrects without a
                // blind re-snapshot.
                let words: Vec<&str> = wanted.split_whitespace().collect();
                let near: Vec<String> = self
                    .evidence
                    .iter()
                    .filter(|reference| {
                        let lower = reference.name.to_lowercase();
                        words.iter().any(|word| word.len() > NEAR_WORD_CHARS && lower.contains(word))
                    })
                    .take(NEAREST_SHOWN)
                    .map(|reference| format!("@{} {} {:?}", reference.handle, reference.role, reference.name))
                    .collect();
                if near.is_empty() {
                    Err(format!(
                        "nothing in the latest page state is named {target:?}; take a snapshot to see what is \
                         there, or act by @ref"
                    ))
                } else {
                    Err(format!(
                        "nothing is named exactly {target:?}; the nearest are {}. Use one of these @refs, or \
                         take a snapshot.",
                        near.join(", ")
                    ))
                }
            }
            many => Err(format!(
                "{target:?} names {} elements: {}",
                many.len(),
                many.iter()
                    .take(AMBIGUOUS_SHOWN)
                    .map(|reference| format!("@{} {} {:?}", reference.handle, reference.role, reference.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The ref the latest render gave the one element of the role and name a
    /// handle of this document named, when the page replaced that element.
    pub(crate) fn renamed(&self, target: &str) -> Option<String> {
        let handle = target.trim().trim_start_matches('@');
        let was = self.issued.get(handle).filter(|was| !was.name.is_empty())?;
        let mut same = self.evidence.iter().filter(|r| r.role == was.role && r.name == was.name);
        match (same.next(), same.next()) {
            (Some(one), None) if one.handle != handle => Some(format!("@{}", one.handle)),
            _ => None,
        }
    }
}

fn fitting<'a>(found: &[&'a Reference], roles: &[&str]) -> Vec<&'a Reference> {
    found.iter().filter(|reference| roles.contains(&reference.role.as_str())).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::nodes;
    use super::*;
    use serde_json::json;

    #[test]
    fn an_element_can_be_named_by_its_words_when_that_is_unambiguous() {
        let mut refs = Refs::default();
        refs.render(&[("main".to_owned(), "page".to_owned(), nodes())], false, true).unwrap();
        assert_eq!(refs.resolve("continue", &[]), Ok(3), "exact, any case");
        assert_eq!(refs.resolve("Docs", &[]), Ok(4), "an exact name wins over one that contains it");
        assert_eq!(refs.resolve("API", &[]), Ok(5), "a part of one name");
        let ambiguous = refs.resolve("doc", &[]).unwrap_err();
        assert!(
            ambiguous.contains("2 elements")
                && ambiguous.contains(&format!("@{} link \"Docs\"", refs.handle_for(4).unwrap())),
            "{ambiguous}"
        );
        assert!(refs.resolve("Sign in", &[]).unwrap_err().contains("nothing"));
        assert!(refs.resolve("@ezz.9", &[]).unwrap_err().contains("not a ref"));
        assert_eq!(refs.resolve("doc", &["link"]), Err(ambiguous.clone()), "a role narrows only to one");
        assert_eq!(refs.resolve("doc", &["button"]), Err(ambiguous), "and never to none");
    }

    #[test]
    fn a_shared_name_is_settled_by_what_the_action_is_for() {
        let mut refs = Refs::default();
        let mut nodes = nodes();
        nodes[1] = serde_json::json!({"nodeId":"2","ignored":false,"role":{"type":"role","value":"searchbox"},"name":{"type":"computedString","value":"Continue"},"parentId":"1","backendDOMNodeId":2});
        refs.render(&[("main".to_owned(), "page".to_owned(), nodes)], false, true).unwrap();
        assert!(refs.resolve("Continue", &[]).is_err(), "a click cannot tell the box from the button");
        assert_eq!(refs.resolve("Continue", &["textbox", "searchbox"]), Ok(2), "typing can");
        assert_eq!(
            refs.resolve("Docs", &["searchbox"]),
            Ok(4),
            "an exact name that fits no role still wins alone"
        );
        assert_eq!(refs.resolve("Cont", &["searchbox"]), Ok(2), "a part of a name, by role");
    }

    #[test]
    fn a_ref_from_the_page_just_left_says_what_it_named_and_where_that_is_now() {
        let mut refs = Refs::default();
        let page = |id: i64| {
            vec![json!({"nodeId":"1","backendDOMNodeId":id,"role":{"value":"link"},
                        "name":{"value":"Zucchini Lasagna"}})]
        };
        refs.render(&[("main".into(), "a".into(), page(1))], false, false).unwrap();
        let was = refs.handles()[0].0.clone();
        refs.forget();
        refs.render(&[("main".into(), "b".into(), page(2))], false, false).unwrap();
        let now = refs.handles()[0].0.clone();
        assert_ne!(was, now, "a new document never reuses a name");

        let said = refs.resolve(&format!("@{was}"), &["link"]).unwrap_err();
        assert!(said.contains("Zucchini Lasagna"), "{said}");
        assert!(said.contains(&format!("@{now}")), "it hands over the name to use: {said}");

        // And when the thing really is gone, it says that rather than a name.
        refs.forget();
        refs.render(
            &[(
                "main".into(),
                "c".into(),
                vec![json!({"nodeId":"1","backendDOMNodeId":3,"role":{"value":"link"},
                            "name":{"value":"Something else"}})],
            )],
            false,
            false,
        )
        .unwrap();
        let gone = refs.resolve(&format!("@{was}"), &["link"]).unwrap_err();
        assert!(gone.contains("Zucchini Lasagna") && gone.contains("nothing in this page"), "{gone}");
    }
}
