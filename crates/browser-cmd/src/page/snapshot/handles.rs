//! Handles: the opaque `@e1a.31` names a document's nodes are given, and which of them still hold.

use std::collections::HashMap;

use super::{Reference, Refs};

impl Refs {
    /// The handle the latest render gave this DOM node, if any — for naming an
    /// element the agent did not ask for (the thing covering its target) back as
    /// a ref it can act on.
    pub(crate) fn handle_for(&self, node: i64) -> Option<String> {
        self.by_node.get(&node).cloned()
    }

    /// The role the latest render gave this DOM node, if it showed it.
    pub(crate) fn role_for(&self, node: i64) -> Option<&str> {
        self.evidence.get(*self.evidence_by_node.get(&node)?).map(|reference| reference.role.as_str())
    }

    pub(crate) fn bind_documents(&mut self, documents: &HashMap<String, Vec<(String, u64)>>) {
        if &self.documents == documents {
            return;
        }
        self.by_node.retain(|_, handle| {
            self.issued.get(handle).is_some_and(|issued| {
                let frame = &issued.frame_id;
                self.documents.get(frame).is_some_and(|previous| Some(previous) == documents.get(frame))
            })
        });
        self.documents = documents.clone();
    }

    /// A tab attached anew numbers its frames' documents afresh; its main
    /// frame, on the load these handles were issued for, holds the same document.
    pub(crate) fn reattached(&mut self, main: &str, documents: &HashMap<String, Vec<(String, u64)>>) {
        if let (Some(known), Some(now)) = (self.documents.get_mut(main), documents.get(main)) {
            known.clone_from(now);
        }
    }

    pub(super) fn issue(
        &mut self,
        backend: i64,
        frame: &str,
        role: &str,
        name: &str,
    ) -> crate::Result<(String, bool)> {
        let mut fresh = false;
        let handle = match self.by_node.entry(backend) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if self.tag.is_none() {
                    self.tag = Some(fresh_tag()?);
                }
                self.next += 1;
                fresh = true;
                entry.insert(format!("{}.{}", self.tag.as_deref().unwrap_or_default(), self.next))
            }
        };
        let reference = Reference {
            handle: handle.clone(),
            node: backend,
            frame_id: frame.to_owned(),
            role: role.to_owned(),
            name: name.to_owned(),
        };
        self.issued.insert(handle.clone(), reference.clone());
        self.evidence_by_node.insert(backend, self.evidence.len());
        self.evidence.push(reference);
        Ok((handle.clone(), fresh))
    }

    /// Each ref the latest render showed, with the DOM node it names, in the
    /// order they appeared — for pinning a viewport coordinate onto each.
    pub(crate) fn handles(&self) -> Vec<(String, i64)> {
        self.evidence.iter().map(|reference| (reference.handle.clone(), reference.node)).collect()
    }
}

/// Refs are opaque, not indices: `<tag>.<n>`, where the tag names one document
/// generation and `n` never restarts inside it. Tags come from one counter per
/// daemon, seeded from OS entropy on first use, so a ref from an earlier
/// document or another session names nothing here rather than whatever now
/// sits in that position, and a replacement daemon most likely starts
/// elsewhere. Short on purpose — every action the agent takes has to copy one
/// back verbatim, and refs are a third of a state's bytes. A tag is `e` then
/// base 36 (`e1a`), so a name such as "v1.2" never reads as a ref.
fn fresh_tag() -> crate::Result<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if NEXT.load(Ordering::Relaxed) == 0 {
        let mut seed = [0u8; 2];
        getrandom::fill(&mut seed)
            .map_err(|_| crate::Error::Tool("cannot allocate refs: system entropy unavailable".into()))?;
        // Three base-36 digits for tens of thousands of documents, so a
        // replacement daemon rarely starts near its predecessor's range.
        let _ = NEXT.compare_exchange(
            0,
            36 * 36 + u64::from(u16::from_be_bytes(seed)) % (36 * 36 * 35),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
    Ok(tag(NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// The tag of document number `n`: `e`, then `n` in base 36.
fn tag(mut n: u64) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((n % 36) as u32, 36).expect("below 36"));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    std::iter::once('e').chain(digits.into_iter().rev()).collect()
}

pub(crate) fn is_handle(target: &str) -> bool {
    let target = target.trim();
    let Some(body) = target.strip_prefix('@').unwrap_or(target).strip_prefix('e') else {
        return false;
    };
    let Some((tag, ordinal)) = body.split_once('.') else { return false };
    !tag.is_empty()
        && tag.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !ordinal.is_empty()
        && ordinal.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{nodes, page};
    use super::*;

    #[test]
    fn refs_are_short_letter_led_and_parse_back_as_handles() {
        assert_eq!(tag(46), "e1a");
        assert_eq!(tag(36 * 36), "e100");
        let mut refs = Refs::default();
        refs.render(&[("main".to_owned(), "page".to_owned(), nodes())], false, true).unwrap();
        let handle = refs.handle_for(3).unwrap();
        let (tag, ordinal) = handle.split_once('.').unwrap();
        assert!((4..=5).contains(&tag.len()) && ordinal.parse::<u64>().is_ok(), "{handle}");
        assert!(is_handle(&handle) && is_handle(&format!("@{handle}")) && is_handle("@e4803152.3"));
        for name in ["Buy now", "3.5", "v1.2", "a1.0", "@e1a", "e1a.", "E1a.1", "@e.1", "@e-1.1"] {
            assert!(!is_handle(name), "{name}");
        }
    }

    #[test]
    fn changed_frame_documents_cannot_reuse_a_ref_even_when_backend_node_ids_repeat() {
        let mut refs = Refs::default();
        let mut tree = page(10);
        tree.push(("child".into(), "child page".into(), page(20).remove(0).2));
        let mut documents = HashMap::from([
            ("main".into(), vec![("main".into(), 1)]),
            ("child".into(), vec![("main".into(), 1), ("child".into(), 2)]),
        ]);
        refs.bind_documents(&documents);
        refs.render(&tree, false, false).unwrap();
        let main = refs.handle_for(10).unwrap();
        let child = refs.handle_for(20).unwrap();
        documents.insert("child".into(), vec![("main".into(), 1), ("child".into(), 3)]);
        refs.bind_documents(&documents);
        refs.render(&tree, false, false).unwrap();
        assert_eq!(refs.handle_for(10).unwrap(), main);
        assert_ne!(refs.handle_for(20).unwrap(), child);
        assert!(refs.resolve(&child, &[]).is_err());
    }

    #[test]
    fn independent_sessions_cannot_resolve_each_others_refs() {
        let mut first = Refs::default();
        let mut second = Refs::default();
        first.render(&page(10), false, false).unwrap();
        second.render(&page(10), false, false).unwrap();
        let reference = first.handle_for(10).unwrap();
        assert_ne!(second.handle_for(10).unwrap(), reference);
        assert!(second.resolve(&reference, &[]).is_err());
        assert_eq!(first.resolve(&reference, &[]), Ok(10));
    }
}
