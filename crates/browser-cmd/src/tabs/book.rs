//! A session's view of its tabs: the one it is on, the ones it holds by
//! handle, the ones it left, and the browser contexts it owns.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::Value;

use crate::browser::host::{Host, Shared};
use crate::chrome::tab::Tab;
use crate::tabs::context::BrowserContext;
use crate::tabs::ledger::{Class, Hold, Ledger, refusal};
use crate::tabs::registry::{Entry, Registry};
use crate::tabs::status::TabStatus;
use crate::tabs::store::{Saved, Store};
use crate::tabs::{target_id, unique};
use crate::{Error, Result, Settings};

#[derive(Default)]
pub(crate) struct Book {
    /// The session's name, which holds its tabs in the browser's ledger.
    pub(crate) name: String,
    pub(crate) ledger: Rc<RefCell<Ledger>>,
    pub(crate) active: Option<Tab>,
    /// Every tab this session opened or adopted, by handle.
    pub(crate) registry: Registry,
    store: Option<Store>,
    /// A browser context of this session's own, after `retry-clean`.
    pub(crate) context: Option<BrowserContext>,
    /// Contexts of its own an earlier clean retry left, while tabs of this session live there.
    pub(crate) retired: Vec<BrowserContext>,
    /// The pages open before the action in progress, so a tab it opens or
    /// closes can be followed.
    pub(crate) before: Option<HashSet<String>>,
    /// The tabs this session left for another, most recent last, to go back
    /// to when the one it is on closes.
    pub(crate) previous: Vec<String>,
    /// The browser this session's tab lived in was replaced, taking the tab:
    /// said once, by the next action.
    pub(crate) lost: bool,
    /// What an earlier daemon left of this session, until it is taken back.
    pub(crate) earlier: Option<Saved>,
    /// A word on the file the tabs came from, said once.
    pub(crate) notice: Option<String>,
}

impl Book {
    /// The tabs a file holds, written back there; none kept when there is no file.
    pub(crate) fn new(host: &Host, name: &str, file: Option<PathBuf>) -> Self {
        let mut book = Self::default();
        (book.name, book.ledger) = (name.into(), crate::tabs::ledger::of(host));
        if let Some(file) = file {
            let (store, mut saved, notice) = Store::open(file);
            book.registry = std::mem::take(&mut saved.registry);
            let owned = saved.context.is_some() || !saved.retired.is_empty();
            book.earlier = (owned || !book.registry.entries.is_empty()).then_some(saved);
            (book.store, book.notice) = (Some(store), notice);
        }
        book
    }

    /// A tab this session opened or followed: held as its own unless it is
    /// borrowed, under a handle, isolated when it lives in a context of its own.
    pub(crate) fn own(&mut self, target: &str) -> &mut Entry {
        let mut ledger = self.ledger.borrow_mut();
        if ledger.class(target, &self.name) != Class::Borrowed {
            ledger.hold(target, Hold::Owned(self.name.clone()));
        }
        let isolated = self.home(target).is_some();
        let entry = self.registry.adopt(target);
        entry.isolated = isolated;
        entry
    }

    /// A tab's handle, given one when it has none.
    pub(crate) fn handle(&mut self, target: &str) -> String {
        self.registry.adopt(target).name()
    }

    /// What a tab now shows, or what became of it.
    pub(crate) fn mark(&mut self, target: &str, status: TabStatus) {
        self.registry.adopt(target).status = status;
    }

    /// A page this session read and left open: its own, kept where it was read, by handle.
    pub(crate) fn keep_opened(
        &mut self,
        target: &str,
        address: &str,
        title: &str,
        status: TabStatus,
    ) -> String {
        self.own(target);
        let name = self.registry.keep(target, address, None).name();
        self.registry.seen(target, address, title, status);
        name
    }

    /// The context of this session's own a page lives in, as the browser last told.
    pub(crate) fn home(&self, target: &str) -> Option<&BrowserContext> {
        self.context.iter().chain(&self.retired).find(|context| {
            let events = context.cdp.events();
            events
                .targets
                .pages()
                .iter()
                .any(|page| target_id(page) == target && page["browserContextId"] == ***context)
        })
    }

    /// Whether a tab of this session that `stays` lives in a context.
    pub(crate) fn lives(&self, context: &str, stays: impl Fn(&Entry) -> bool) -> bool {
        let entries = self.registry.entries.iter().filter(|entry| stays(entry));
        entries.filter_map(|entry| self.home(&entry.target)).any(|home| **home == *context)
    }

    /// Write the tabs back, once what an earlier daemon left was taken back.
    pub(crate) fn save(&mut self) {
        let Some(store) = self.store.as_mut().filter(|_| self.earlier.is_none()) else { return };
        store.save(Saved {
            registry: self.registry.clone(),
            active: self.active.as_ref().map(|tab| tab.target.clone()),
            previous: self.previous.clone(),
            context: self.context.as_deref().map(str::to_owned),
            retired: self.retired.iter().map(|context| context.to_string()).collect(),
            ..Saved::default()
        });
    }

    /// The active tab, while it is attached to the browser now running.
    pub(crate) fn live(&self, shared: &Shared) -> Option<Tab> {
        self.active.clone().filter(|tab| Rc::ptr_eq(&tab.cdp, &shared.cdp) && tab.attached())
    }

    /// A tab to read without making it the active one: the active one when it
    /// is that tab and live, else attached anew.
    pub(crate) async fn reach(&self, shared: &Shared, settings: &Settings, target: &str) -> Result<Tab> {
        match self.live(shared).filter(|tab| tab.target == target) {
            Some(tab) => Ok(tab),
            None => shared.attach(settings, target.into()).await,
        }
    }

    /// Make a tab the active one, or none, remembering the one left, while it
    /// is still this session's, to go back to; the targets left and arrived at.
    pub(crate) fn switch(&mut self, tab: Option<Tab>) -> (Option<String>, Option<String>) {
        if let Some(tab) = &tab {
            self.own(&tab.target);
        }
        if let Some(left) = &self.active
            && left.attached()
            && self.registry.of(&left.target).is_some()
        {
            self.previous.retain(|target| *target != left.target);
            self.previous.push(left.target.clone());
        }
        let (left, next) =
            (self.active.as_ref().map(|t| t.target.clone()), tab.as_ref().map(|t| t.target.clone()));
        self.previous.retain(|target| Some(target) != next.as_ref());
        self.active = tab;
        (left, next)
    }

    /// The tab to go to once the active one is gone: the one left most
    /// recently that is open and still this session's, else its first one open.
    pub(crate) fn next(&mut self, open: &HashSet<String>) -> Option<String> {
        let ours = |target: &String| open.contains(target) && self.registry.of(target).is_some();
        while let Some(target) = self.previous.pop() {
            if ours(&target) {
                return Some(target);
            }
        }
        self.registry.entries.iter().map(|entry| &entry.target).find(|target| open.contains(*target)).cloned()
    }

    /// The same tab, now another target: a clean retry, a restored page. The
    /// active one gone so is the first to go back to.
    pub(crate) fn retarget(&mut self, from: &str, to: &str) {
        self.registry.retarget(from, to);
        for target in self.previous.iter_mut().filter(|target| *target == from) {
            *target = to.into();
        }
        if self.active.as_ref().is_some_and(|tab| tab.target == from) {
            self.previous.retain(|target| target != to);
            self.previous.push(to.into());
        }
    }

    /// The tab a handle — `t3` or `3` — names, or a unique prefix of the
    /// browser id of a tab this session holds, or the current one when none is named.
    pub(crate) fn chosen(&self, pages: &[Value], id: Option<&str>) -> Result<String> {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return self
                .active
                .as_ref()
                .map(|tab| tab.target.clone())
                .ok_or_else(|| Error::Tool("this session has no tab open; name one from tab list".into()));
        };
        let open = |target: &str| pages.iter().any(|page| crate::tabs::target_id(page) == target);
        if let Some(entry) = self.registry.named(id) {
            return match open(&entry.target) {
                true => Ok(entry.target.clone()),
                false => Err(Error::Tool(format!("{id} is no longer open; run tab list"))),
            };
        }
        if id.strip_prefix('t').unwrap_or(id).parse::<u32>().is_ok() {
            return Err(Error::Tool(format!("this session has no tab {id}; run tab list")));
        }
        let target = unique(pages, id)?;
        match self.registry.of(&target) {
            Some(_) => Ok(target),
            None => refusal(self.ledger.borrow().class(&target, &self.name), id).map_or(Ok(target), Err),
        }
    }
}

impl Drop for Book {
    /// A session ending lets go of every page it holds.
    fn drop(&mut self) {
        self.ledger.borrow_mut().release_all(&self.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_next_tab_is_the_one_left_last_still_open_and_this_sessions_else_the_first_open() {
        let mut book = Book::default();
        for target in ["A", "B", "C"] {
            book.registry.adopt(target);
        }
        book.previous = ["A", "X", "C", "B"].map(String::from).to_vec();
        let open = HashSet::from(["A", "C", "X"].map(String::from));
        assert_eq!(book.next(&open).as_deref(), Some("C"), "B is closed");
        assert_eq!(book.next(&open).as_deref(), Some("A"), "X is not this session's");
        assert_eq!(book.next(&open).as_deref(), Some("A"), "none left: the first open");
        assert!(book.next(&HashSet::new()).is_none());
    }

    #[test]
    fn a_file_with_contexts_of_its_own_and_no_tab_left_is_still_taken_back() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("tabs.json");
        std::fs::write(&file, r#"{"version":1,"next":2,"entries":[],"retired":["R"]}"#).unwrap();
        let book = Book::new(&Host::new(Settings::new(temp.path(), temp.path())), "s", Some(file));
        assert_eq!(book.earlier.as_ref().map(|earlier| earlier.retired.clone()), Some(vec!["R".into()]));
    }

    #[test]
    fn a_number_names_a_handle_and_only_four_hex_digits_or_more_name_an_id() {
        let mut book = Book::default();
        let pages = [json!({"targetId":"12AB34CD"}), json!({"targetId":"1234FFFF"})];
        book.registry.adopt("12AB34CD");
        book.ledger.borrow_mut().hold("1234FFFF", Hold::Owned(book.name.clone()));
        let chosen = |word: &str| book.chosen(&pages, Some(word)).map_err(|error| error.to_string());
        assert_eq!(chosen("t1").as_deref(), Ok("12AB34CD"));
        assert_eq!(chosen("1").as_deref(), Ok("12AB34CD"));
        assert_eq!(chosen("12AB").as_deref(), Ok("12AB34CD"));
        assert_eq!(chosen("1234F").as_deref(), Ok("1234FFFF"));
        for (word, said) in [
            ("12", "no tab 12;"),
            ("t12", "no tab t12;"),
            ("1234", "no tab 1234;"),
            ("12A", "no open tab is 12A"),
        ] {
            assert!(chosen(word).unwrap_err().contains(said), "{word}");
        }
    }
}
