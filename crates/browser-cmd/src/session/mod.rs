//! One agent's session: its tab, that tab's document, the hand it acts with,
//! its output, and the memory that tells it when it is going in circles.

mod after;
mod awareness;
pub(crate) mod document;
pub(crate) mod land;
pub(crate) mod notes;
mod output;
mod pdf;
mod state;
mod visual;
pub(crate) mod wall;

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde_json::Value;

use crate::browser::host::Host;
use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::tabs::book::Book;
use crate::{Error, Result, Settings};

pub(crate) use after::after;
use awareness::Awareness;
use document::Document;
use output::Output;
pub(crate) use output::{Image, Outcome};

/// What a command does with a session and its arguments.
pub(crate) type Run =
    for<'a> fn(&'a mut Session, Value) -> Pin<Box<dyn Future<Output = Result<String>> + 'a>>;

/// How many left tabs' documents a session keeps.
const PARKED: usize = 16;

pub(crate) struct Session {
    pub(crate) host: Rc<Host>,
    pub(crate) book: Book,
    pub(crate) document: Document,
    /// What this session knew of the documents of tabs it left, by target, so
    /// coming back to one says what changed there rather than all of it.
    parked: std::collections::HashMap<String, Document>,
    pub(crate) hand: Hand,
    pub(crate) output: Output,
    awareness: Awareness,
    /// When its last command finished, so an idle session can be let go.
    pub(crate) used: std::time::Instant,
    /// `close` ended it: its slot is free.
    pub(crate) ended: bool,
}

impl Host {
    /// A session of its own on this browser, under a name of its own, kept
    /// nowhere: a tab and a set of handles.
    #[cfg(test)]
    pub(crate) fn session(self: &Rc<Self>) -> Session {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        let name = format!("test{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Session::new(self.clone(), &name, None)
    }

    /// A caller's session, whose tabs are kept in the workspace under its name.
    #[must_use]
    pub(crate) fn named_session(self: &Rc<Self>, name: &str) -> Session {
        let file = crate::tabs::store::path(self.settings(), name);
        Session::new(self.clone(), name, Some(file))
    }
}

impl Session {
    pub(crate) fn new(host: Rc<Host>, name: &str, file: Option<std::path::PathBuf>) -> Self {
        let settings = host.settings();
        Self {
            awareness: Awareness::new(settings.awareness),
            hand: Hand::new(settings.pace),
            book: Book::new(&host, name, file),
            document: Document::new(),
            parked: std::collections::HashMap::new(),
            output: Output::new(),
            used: std::time::Instant::now(),
            ended: false,
            host,
        }
    }

    /// Run one command the caller has looked up: its name, whether it is meant
    /// to move the page, whether it acts on a tab, and what it does.
    pub(crate) async fn perform(
        &mut self,
        name: &str,
        changing: bool,
        acting: bool,
        run: Run,
        args: Value,
    ) -> Outcome {
        self.output.picture = None;
        let revision = self.document.refs.revision();
        let asked = args.to_string();
        // What is open before an action that may open or close a tab; after()
        // follows it, and a command that ends without after() is followed here.
        let restored = crate::tabs::begin(self, changing, acting).await;
        let mut result = run(self, args).await;
        if let (Ok(output), Some(tab)) = (&mut result, self.book.active.clone())
            && let Ok((_, notes)) = crate::tabs::follow(self, &tab).await
        {
            output.extend(notes.iter().map(|note| format!("\n{note}")));
        }
        // Only an action meant to move the page enters the trajectory memory; a
        // re-look (snapshot, text) is not progress and must not read as a loop.
        let awareness = match changing {
            true => {
                self.awareness_note((name, asked), result.is_ok(), self.document.refs.revision() != revision)
            }
            false => None,
        };
        let mut output = match &result {
            Ok(output) => output.clone(),
            Err(error) => format!("Error: {error}"),
        };
        if matches!(result, Err(Error::Unacknowledged(_)))
            && let Some((notes, next)) = crate::tabs::close::replace_hung(self).await
        {
            output.extend(notes.iter().map(|note| format!("\n{}", notes::event(note))));
            let then = match next {
                Some(url) => format!("switched to {}", crate::web::shown(&url)),
                None => "the next command opens a fresh tab".into(),
            };
            output.push_str(&format!(
                "\nThe tab stopped answering and was closed; {then}, so navigate to the page again or take \
                 another route."
            ));
        }
        output.extend(restored.iter().map(|note| format!("\n{note}")));
        self.book.save();
        self.used = std::time::Instant::now();
        if let Some(awareness) = awareness {
            output.push_str("\n\n");
            output.push_str(&awareness);
        }
        match result {
            Ok(_) => Outcome {
                ok: true,
                output,
                images: self.output.picture.take().into_iter().collect(),
                error: None,
            },
            Err(error) => Outcome::failed(output, error.kind()),
        }
    }

    fn awareness_note(&mut self, action: (&str, String), ok: bool, captured: bool) -> Option<String> {
        let settled =
            self.book.active.as_ref().is_some_and(|tab| {
                tab.cdp.events().navigation_quiet(&tab.session, self.settings().waits.quiet)
            });
        self.awareness.observe(action, ok, self.document.signature.filter(|_| captured && settled))
    }

    pub(crate) fn settings(&self) -> &Settings {
        self.host.settings()
    }

    /// Wait for the page to come to rest after an input that may move it.
    pub(crate) async fn settle(&mut self, tab: &Tab) -> crate::page::settle::Settled {
        crate::page::settle::settle(&mut self.document.world, tab, &self.host.settings().waits).await
    }

    /// A saved file's path as the agent names it: from the workspace.
    pub(crate) fn relative(&self, path: &std::path::Path) -> String {
        path.strip_prefix(&self.settings().workspace).unwrap_or(path).display().to_string()
    }

    /// This session's tab, opened on first use; a tab it did not already have
    /// comes with a document nothing is known about yet.
    pub(crate) async fn current(&mut self) -> Result<Tab> {
        let (tab, fresh) = crate::tabs::current(self).await?;
        if fresh {
            self.document.forget();
        }
        Ok(tab)
    }

    /// The node a ref or a visible name addresses in the current document.
    pub(crate) fn node(&self, target: &str) -> Result<i64> {
        self.document.resolve(self.book.active.as_ref(), target, &[])
    }

    /// An element to put text into.
    pub(crate) fn field(&self, target: &str) -> Result<i64> {
        self.document.resolve(
            self.book.active.as_ref(),
            target,
            &["textbox", "searchbox", "combobox", "spinbutton"],
        )
    }

    pub(crate) fn resolve(&self, target: &str, roles: &[&str]) -> Result<i64> {
        self.document.resolve(self.book.active.as_ref(), target, roles)
    }
}

impl crate::tabs::Holder for Session {
    fn book(&mut self) -> &mut Book {
        &mut self.book
    }

    fn host(&self) -> Rc<Host> {
        self.host.clone()
    }

    fn switched(&mut self, left: Option<String>, next: Option<String>) {
        if left == next {
            self.document.forget();
            return;
        }
        let mut known = next.and_then(|target| self.parked.remove(&target)).unwrap_or_else(Document::new);
        known.told = std::mem::take(&mut self.document.told);
        let leaving = std::mem::replace(&mut self.document, known);
        if let Some(target) = left {
            if self.parked.len() >= PARKED {
                self.parked.clear();
            }
            self.parked.insert(target, leaving);
        }
    }
}
