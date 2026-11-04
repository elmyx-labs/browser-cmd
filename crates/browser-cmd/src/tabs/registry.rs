//! The tabs a session opened or adopted, each by a handle it keeps for the
//! session's life (t1, t2, …): which hold evidence and what each last showed.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::tabs::status::TabStatus;
use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) handle: u32,
    pub(crate) target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    /// The address it was kept at; none when it is not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kept: Option<String>,
    /// Somebody else's page this session borrowed: never closed, given back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) borrowed: bool,
    /// It lives in the session's own browser context, not the profile's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) isolated: bool,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
}

impl Entry {
    pub(crate) fn name(&self) -> String {
        format!("t{}", self.handle)
    }

    /// Kept, and not to be reopened by itself.
    pub(crate) fn gone(&self) -> bool {
        self.status.gone()
    }

    /// Kept, and no longer at the address it was kept at.
    pub(crate) fn moved(&self) -> bool {
        self.kept.as_deref().is_some_and(|kept| !same_page(kept, &self.url))
    }

    /// What is wrong with a kept tab as last seen, if anything.
    pub(crate) fn fault(&self) -> Option<String> {
        self.kept.as_ref()?;
        match self.status {
            TabStatus::Ok => self.moved().then(|| "moved".into()),
            ref status => Some(status.to_string()),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Registry {
    /// The last handle given: one is never given twice.
    pub(super) next: u32,
    pub(crate) entries: Vec<Entry>,
}

impl Registry {
    /// The entry for a target, given the next handle when it has none.
    pub(crate) fn adopt(&mut self, target: &str) -> &mut Entry {
        let at = match self.entries.iter().position(|entry| entry.target == target) {
            Some(at) => at,
            None => {
                self.next += 1;
                self.entries.push(Entry { handle: self.next, target: target.into(), ..Entry::default() });
                self.entries.len() - 1
            }
        };
        &mut self.entries[at]
    }

    pub(crate) fn of(&self, target: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.target == target)
    }

    /// The entry a word names: `t3`, or just `3`.
    pub(crate) fn named(&self, word: &str) -> Option<&Entry> {
        let handle: u32 = word.strip_prefix('t').unwrap_or(word).parse().ok()?;
        self.entries.iter().find(|entry| entry.handle == handle)
    }

    pub(crate) fn kept(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| entry.kept.is_some())
    }

    /// Keep a tab at the address it is at, under a label when one is given.
    pub(crate) fn keep(&mut self, target: &str, url: &str, label: Option<String>) -> &Entry {
        let entry = self.adopt(target);
        entry.kept = Some(url.into());
        entry.url = url.into();
        entry.label = label.or(entry.label.take());
        entry
    }

    pub(crate) fn unkeep(&mut self, target: &str) -> &Entry {
        let entry = self.adopt(target);
        (entry.kept, entry.label) = (None, None);
        entry
    }

    /// The same tab, now in another target of this session's own: a clean
    /// retry, a restored page.
    pub(crate) fn retarget(&mut self, from: &str, to: &str) {
        match self.entries.iter_mut().find(|entry| entry.target == from) {
            Some(entry) => (entry.target, entry.borrowed) = (to.into(), false),
            None => _ = self.adopt(to),
        }
    }

    pub(crate) fn forget(&mut self, target: &str) {
        self.entries.retain(|entry| entry.target != target);
    }

    /// Drop the tabs that are gone and were not kept; a kept one is restored instead.
    pub(crate) fn prune(&mut self, open: &HashSet<String>) {
        self.entries.retain(|entry| entry.kept.is_some() || open.contains(&entry.target));
    }

    /// After a daemon restart: the tabs not open are forgotten, or, when kept,
    /// gone from the earlier run — gone for good when they lived in a context
    /// of the session's own that is gone too.
    pub(crate) fn resume(&mut self, open: &HashSet<String>, context: bool) {
        self.prune(open);
        for entry in self.entries.iter_mut().filter(|entry| !open.contains(&entry.target)) {
            entry.status = if entry.isolated && !context { TabStatus::Lost } else { TabStatus::Earlier };
        }
    }

    /// The kept tabs to reopen, by target and kept address: not open and not
    /// gone. One that lived in the session's own context, when that is gone,
    /// is gone for good instead.
    pub(crate) fn reopenable(&mut self, open: &HashSet<String>, context: bool) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for entry in self.entries.iter_mut().filter(|entry| !open.contains(&entry.target) && !entry.gone()) {
            let Some(kept) = entry.kept.clone() else { continue };
            match entry.isolated && !context {
                true => entry.status = TabStatus::Lost,
                false => found.push((entry.target.clone(), kept)),
            }
        }
        found
    }

    /// `tab restore`: the kept tabs an earlier run left gone, the one named or
    /// all, made reopenable.
    pub(crate) fn revive(&mut self, word: Option<&str>) -> Result<()> {
        let named = match word {
            Some(word) => Some(
                self.named(word)
                    .ok_or_else(|| Error::Tool(format!("this session has no tab {word}; run tab list")))?,
            ),
            None => None,
        };
        if let Some(entry) = named.filter(|entry| entry.status == TabStatus::Lost) {
            return Err(Error::Tool(format!(
                "{} lived in a browser context of this session's own that is gone; it is not reopened in another",
                entry.name()
            )));
        }
        let handle = named.map(|entry| entry.handle);
        let mut revived = false;
        for entry in self.entries.iter_mut() {
            if entry.status == TabStatus::Earlier && handle.is_none_or(|handle| handle == entry.handle) {
                entry.status = TabStatus::Unchecked;
                revived = true;
            }
        }
        match revived {
            true => Ok(()),
            false => Err(Error::Tool("no kept tab is gone since an earlier run; nothing to restore".into())),
        }
    }

    /// What a tab now shows; a note when this took a kept tab off its kept page.
    pub(crate) fn seen(&mut self, target: &str, url: &str, title: &str, status: TabStatus) -> Option<String> {
        let entry = self.adopt(target);
        let left =
            entry.kept.as_deref().is_some_and(|kept| same_page(kept, &entry.url) && !same_page(kept, url));
        entry.url = url.into();
        entry.title = title.into();
        // An error page says no more than that; the navigation said which error.
        if status != TabStatus::Error || !matches!(entry.status, TabStatus::Failed(_)) {
            entry.status = status;
        }
        left.then(|| {
            format!(
                "{} (kept) left its kept page; `navigate back` returns, `tab keep` re-keeps here.",
                entry.name()
            )
        })
    }
}

/// One page, whatever its fragment says.
pub(crate) fn same_page(a: &str, b: &str) -> bool {
    a.split('#').next() == b.split('#').next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_tab_that_leaves_its_page_is_said_once_and_gone_tabs_are_pruned_unless_kept() {
        let mut registry = Registry::default();
        let entry = registry.adopt("K");
        entry.kept = Some("https://k.test/a".into());
        entry.url = "https://k.test/a".into();
        assert!(
            registry.seen("K", "https://k.test/a#b", "K", TabStatus::Ok).is_none(),
            "a fragment is the same page"
        );
        let left = registry.seen("K", "https://k.test/next", "K", TabStatus::Ok).unwrap();
        assert!(left.starts_with("t1 (kept) left its kept page"), "{left}");
        assert!(registry.seen("K", "https://k.test/more", "K", TabStatus::Ok).is_none(), "said once");
        assert!(registry.of("K").unwrap().moved());
        registry.adopt("N");
        registry.prune(&HashSet::new());
        assert_eq!(registry.entries.iter().map(Entry::name).collect::<Vec<_>>(), ["t1"]);
        registry.retarget("K", "K2");
        assert_eq!(registry.of("K2").unwrap().handle, 1, "a restored tab keeps its handle");
    }

    #[test]
    fn a_kept_tab_is_reopened_only_in_the_kind_of_context_it_lived_in() {
        let mut registry = Registry::default();
        for (target, isolated) in [("D", false), ("I", true), ("O", true)] {
            registry.keep(target, &format!("https://{target}.test/"), None);
            registry.adopt(target).isolated = isolated;
        }
        registry.adopt("N");
        let open = HashSet::from(["O".to_owned()]);
        let mut after = Registry { entries: registry.entries.clone(), next: registry.next };
        after.resume(&open, false);
        let status = |registry: &Registry, word: &str| registry.named(word).unwrap().status.clone();
        assert_eq!((status(&after, "t1"), status(&after, "t2")), (TabStatus::Earlier, TabStatus::Lost));
        assert_eq!(status(&after, "t3"), TabStatus::Unchecked, "an open tab is taken back as it was");
        assert!(after.named("t4").is_none(), "a tab not kept and not open is forgotten");
        assert!(after.reopenable(&open, true).is_empty(), "gone tabs wait for tab restore");
        assert!(after.revive(Some("t2")).unwrap_err().to_string().contains("not reopened in another"));
        after.revive(None).unwrap();
        assert_eq!(after.reopenable(&open, true), [("D".into(), "https://D.test/".into())]);
        assert!(after.revive(None).is_err(), "nothing left to restore");

        let mut same = registry;
        assert_eq!(same.reopenable(&open, true).len(), 2, "within one run both come back");
        assert_eq!(same.reopenable(&open, false), [("D".into(), "https://D.test/".into())]);
        assert_eq!(status(&same, "t2"), TabStatus::Lost, "never reopened in the profile's context");
    }
}
