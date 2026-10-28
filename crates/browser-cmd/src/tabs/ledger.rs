//! Who holds each page, and every claim on one: this session's own or
//! borrowed, another session's, nobody's, or nobody's for long. A page held by
//! another session is refused by name; one nobody holds is taken only by
//! `tab use`, never implicitly. One ledger per host, across its connections.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use serde_json::Value;

use crate::browser::host::{BrowserProcess, Host, Shared};
use crate::chrome::tab::Tab;
use crate::tabs::book::Book;
use crate::tabs::{Holder, attach, leave, target_id, unique};
use crate::{Error, Result};

/// Who holds a page: a session that opened it, one that borrowed it, or
/// nobody for long, as a page read and closed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hold {
    Owned(String),
    Borrowed(String),
    Transient,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Class {
    Own,
    Borrowed,
    Other(String),
    Unowned,
    Transient,
}

#[derive(Default)]
pub(crate) struct Ledger {
    holds: HashMap<String, Hold>,
    /// The pages already looked at for an opener a session holds.
    seen: HashSet<String>,
    /// The Chrome launched here whose blank startup page was given out.
    startup: Weak<BrowserProcess>,
}

/// The host's ledger, made on first use.
pub(crate) fn of(host: &Host) -> Rc<RefCell<Ledger>> {
    let above = host.above.get_or_init(|| Rc::new(RefCell::new(Ledger::default())));
    above.clone().downcast().expect("a host keeps only the tabs ledger")
}

impl Ledger {
    pub(crate) fn class(&self, target: &str, session: &str) -> Class {
        match self.holds.get(target) {
            None => Class::Unowned,
            Some(Hold::Transient) => Class::Transient,
            Some(Hold::Owned(holder)) if holder == session => Class::Own,
            Some(Hold::Borrowed(holder)) if holder == session => Class::Borrowed,
            Some(Hold::Owned(holder) | Hold::Borrowed(holder)) => Class::Other(holder.clone()),
        }
    }

    pub(crate) fn hold(&mut self, target: &str, hold: Hold) {
        self.holds.insert(target.into(), hold);
    }

    pub(crate) fn release(&mut self, target: &str) {
        self.holds.remove(target);
    }

    /// Every page a session holds, let go when the session ends.
    pub(crate) fn release_all(&mut self, session: &str) {
        self.holds.retain(|_, hold| !matches!(hold, Hold::Owned(s) | Hold::Borrowed(s) if s == session));
    }

    /// The pages gone are forgotten; a page first seen now that a page a
    /// session holds opened, directly or down a chain of openers, is that session's.
    pub(crate) fn sweep(&mut self, pages: &[Value]) {
        let open: HashSet<&str> = pages.iter().map(target_id).collect();
        self.holds.retain(|target, _| open.contains(target.as_str()));
        self.seen.retain(|target| open.contains(target.as_str()));
        let mut fresh: Vec<&Value> =
            pages.iter().filter(|page| self.seen.insert(target_id(page).to_owned())).collect();
        while let Some((at, session)) =
            fresh.iter().enumerate().find_map(|(at, page)| Some((at, self.opener(page)?)))
        {
            self.holds.entry(target_id(fresh.remove(at)).to_owned()).or_insert(Hold::Owned(session));
        }
    }

    /// The session holding the page that opened this one.
    fn opener(&self, page: &Value) -> Option<String> {
        match self.holds.get(page["openerId"].as_str()?)? {
            Hold::Owned(session) | Hold::Borrowed(session) => Some(session.clone()),
            Hold::Transient => None,
        }
    }

    /// The blank page a Chrome launched here opened, once per launch, for the first session that asks.
    pub(crate) fn startup(&mut self, shared: &Shared, session: &str) -> Option<String> {
        let launched = Rc::downgrade(shared.process.as_ref()?);
        if Weak::ptr_eq(&self.startup, &launched) {
            return None;
        }
        self.startup = launched;
        let blank = shared
            .cdp
            .events()
            .targets
            .pages()
            .iter()
            .find(|page| page["url"] == "about:blank" && !self.holds.contains_key(target_id(page)))
            .map(|page| target_id(page).to_owned())?;
        self.hold(&blank, Hold::Owned(session.into()));
        Some(blank)
    }
}

/// Why a page named by `word` cannot be taken: another session holds it, or
/// nobody does and it must be borrowed first.
pub(crate) fn refusal(class: Class, word: &str) -> Option<Error> {
    match class {
        Class::Own | Class::Borrowed => None,
        Class::Other(owner) => Some(Error::Tool(format!(
            "{word} is session {owner}'s tab; that session can hand it over with `tab give tN to=SESSION`"
        ))),
        Class::Unowned => {
            Some(Error::Tool(format!("{word} is not this session's tab; `tab use {word}` borrows it")))
        }
        Class::Transient => Some(Error::Tool(format!("no open tab is {word}; run tab list"))),
    }
}

/// The open pages that are not this session's: the sessions holding the
/// others, one name per page, and the ones nobody holds.
pub(crate) fn strangers<'a>(pages: &'a [Value], book: &Book) -> (Vec<String>, Vec<&'a Value>) {
    let ledger = book.ledger.borrow();
    let (mut others, mut unowned) = (Vec::new(), Vec::new());
    for page in pages.iter().filter(|page| book.registry.of(target_id(page)).is_none()) {
        match ledger.class(target_id(page), &book.name) {
            Class::Other(owner) => others.push(owner),
            Class::Unowned => unowned.push(page),
            _ => {}
        }
    }
    (others, unowned)
}

/// Pages this session came to hold without opening them — a window one of
/// its tabs opened after the action, a tab handed over — taken under a handle
/// and said once.
pub(crate) async fn arrivals(holder: &mut impl Holder) -> Vec<String> {
    let Some(shared) = holder.host().running().await else { return Vec::new() };
    let pages = shared.cdp.events().targets.pages().to_vec();
    let book = holder.book();
    let came: Vec<(String, bool, bool, String, bool)> = {
        let mut ledger = book.ledger.borrow_mut();
        ledger.sweep(&pages);
        pages
            .iter()
            .filter(|page| book.registry.of(target_id(page)).is_none())
            .filter_map(|page| {
                let borrowed = match ledger.class(target_id(page), &book.name) {
                    Class::Own => false,
                    Class::Borrowed => true,
                    _ => return None,
                };
                let isolated = book.home(target_id(page)).is_some();
                let opened =
                    page["openerId"].as_str().is_some_and(|opener| book.registry.of(opener).is_some());
                let url = crate::web::shown(page["url"].as_str().unwrap_or_default());
                Some((target_id(page).to_owned(), borrowed, isolated, url, opened))
            })
            .collect()
    };
    came.into_iter()
        .map(|(target, borrowed, isolated, url, opened)| {
            let entry = book.registry.adopt(&target);
            (entry.borrowed, entry.isolated) = (borrowed, isolated);
            match opened {
                true => format!("A new tab {} opened at {url}.", entry.name()),
                false => format!("Tab {} at {url} was handed to this session.", entry.name()),
            }
        })
        .collect()
}

/// `tab use`: borrow a page nobody holds — by its number in `tab list`'s
/// block of such pages, or its id — and make it the active tab.
pub(crate) async fn borrow(holder: &mut impl Holder, pages: &[Value], word: &str) -> Result<Tab> {
    let book = holder.book();
    let (_, unowned) = strangers(pages, book);
    let target = match word.parse::<usize>() {
        Ok(n) => target_id(n.checked_sub(1).and_then(|at| unowned.get(at)).ok_or_else(|| {
            Error::Tool(format!("tab list shows no tab {n} that nobody holds; run tab list"))
        })?)
        .to_owned(),
        Err(_) => match book.registry.named(word) {
            Some(entry) => entry.target.clone(),
            None => unique(pages, word)?,
        },
    };
    let held = book.ledger.borrow().class(&target, &book.name);
    match held {
        Class::Unowned => {
            book.ledger.borrow_mut().hold(&target, Hold::Borrowed(book.name.clone()));
            book.registry.adopt(&target).borrowed = true;
        }
        class => {
            if let Some(refused) = refusal(class, word) {
                return Err(refused);
            }
        }
    }
    attach(holder, &target).await
}

/// `tab give`: hand one of this session's tabs to another session, which
/// takes it on its next command; where this session is now, when it was the
/// active one.
pub(crate) async fn give(holder: &mut impl Holder, target: &str, to: &str) -> Result<Option<String>> {
    let book = holder.book();
    let hold = match book.registry.of(target).is_some_and(|entry| entry.borrowed) {
        true => Hold::Borrowed(to.into()),
        false => Hold::Owned(to.into()),
    };
    book.ledger.borrow_mut().hold(target, hold);
    book.registry.forget(target);
    leave(holder, target).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_page_has_one_class_and_only_a_sessions_own_or_borrowed_pages_are_taken_by_name() {
        let mut ledger = Ledger::default();
        ledger.hold("A", Hold::Owned("a".into()));
        ledger.hold("B", Hold::Borrowed("a".into()));
        ledger.hold("P", Hold::Transient);
        assert_eq!(ledger.class("A", "a"), Class::Own);
        assert_eq!(ledger.class("B", "a"), Class::Borrowed);
        assert_eq!(ledger.class("A", "b"), Class::Other("a".into()));
        assert_eq!(ledger.class("B", "b"), Class::Other("a".into()), "a borrowed page is not free");
        assert_eq!(ledger.class("U", "a"), Class::Unowned);
        assert_eq!(ledger.class("P", "a"), Class::Transient);
        assert!(refusal(ledger.class("A", "a"), "A").is_none());
        let other = refusal(ledger.class("A", "b"), "A1B2").unwrap().to_string();
        assert!(other.contains("session a's tab") && other.contains("tab give"), "{other}");
        let free = refusal(Class::Unowned, "C3").unwrap().to_string();
        assert!(free.contains("`tab use C3` borrows it"), "{free}");
        assert!(refusal(Class::Transient, "P").unwrap().to_string().contains("no open tab is P"));
    }

    #[test]
    fn a_page_first_seen_opened_by_a_held_page_is_its_sessions_down_a_chain() {
        let page = |id: &str, opener: Option<&str>| json!({"targetId": id, "openerId": opener});
        let mut ledger = Ledger::default();
        ledger.sweep(&[page("A", None), page("U", None), page("O", Some("U"))]);
        ledger.hold("A", Hold::Borrowed("s".into()));
        ledger.hold("U", Hold::Owned("s".into()));
        ledger.sweep(&[
            page("A", None),
            page("U", None),
            page("C", Some("B")),
            page("B", Some("A")),
            page("O", Some("U")),
            page("V", Some("W")),
        ]);
        assert_eq!(ledger.class("B", "s"), Class::Own);
        assert_eq!(ledger.class("C", "s"), Class::Own, "a chain of openers, in any order");
        assert_eq!(ledger.class("O", "s"), Class::Unowned, "a page seen before it was held stays nobody's");
        assert_eq!(ledger.class("V", "s"), Class::Unowned, "a page nobody holds gives its popups to nobody");
        ledger.hold("V", Hold::Borrowed("t".into()));
        ledger.sweep(&[page("A", None), page("B", Some("A")), page("V", None)]);
        assert_eq!(ledger.class("C", "s"), Class::Unowned, "a page gone is forgotten");
        assert!(!ledger.seen.contains("C") && ledger.seen.len() == 3);
        ledger.release_all("s");
        assert_eq!((ledger.class("A", "s"), ledger.class("B", "s")), (Class::Unowned, Class::Unowned));
        assert_eq!(ledger.class("V", "s"), Class::Other("t".into()), "another session's stay");
    }
}
