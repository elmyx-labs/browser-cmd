//! What a session knows about the document its tab shows, forgotten together
//! when the document changes.

use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use crate::chrome::cdp::Cdp;
use crate::chrome::tab::Tab;
use crate::page::snapshot::Refs;
use crate::page::world::World;
use crate::text::listed;
use crate::{Error, Result};

/// The frames the refs of the last state were bound to, by the document each
/// frame showed, so a ref from a replaced document is refused.
pub(crate) struct ReferenceScope {
    pub(crate) cdp: Weak<Cdp>,
    pub(crate) session: String,
    pub(crate) frames: HashMap<String, Vec<(String, u64)>>,
}

pub(crate) struct Document {
    pub(crate) world: World,
    pub(crate) caller_world: World,
    pub(crate) refs: Refs,
    pub(crate) scope: Option<ReferenceScope>,
    /// The load that made the document the refs belong to.
    pub(crate) loader: Option<String>,
    /// The latest state's [`Refs::signature`].
    pub(crate) signature: Option<u64>,
    /// The form last sent from this document, so it is not sent twice by mistake.
    pub(crate) submitted: Option<String>,
    /// The load whose PDF bytes were already saved, so they are saved once.
    pub(crate) saved_pdf: Option<String>,
    /// Every line of page text as it read when this session last acted, so what
    /// an action made the page say can be told apart from what it already said.
    pub(crate) said: HashSet<String>,
    /// The load the last action ended on: the first action on another is told
    /// its canvases afresh.
    pub(crate) seen_load: Option<String>,
    /// The canvases the last action ended with, so a change of them is told.
    pub(crate) canvases: Option<String>,
    /// The text the canvases drew as last told, when that is recorded.
    pub(crate) drawn: HashSet<String>,
    /// The address and title this tab's last state showed, said again only when either changes.
    pub(crate) named: Option<(String, String)>,
    /// The tree this document's refs were last shown in whole.
    pub(crate) printed: Option<String>,
    /// The notes this session was already told in full: the local date and
    /// zone, said once, and the long ones whose short form says enough after.
    /// Per session, not per document: they are not the page's.
    pub(crate) told: HashSet<&'static str>,
}

impl Document {
    pub(crate) fn new() -> Self {
        Self {
            world: World::new(),
            caller_world: World::caller(),
            refs: Refs::default(),
            scope: None,
            loader: None,
            signature: None,
            submitted: None,
            saved_pdf: None,
            said: HashSet::new(),
            seen_load: None,
            canvases: None,
            drawn: HashSet::new(),
            named: None,
            printed: None,
            told: HashSet::new(),
        }
    }

    pub(crate) fn resolve(&self, tab: Option<&Tab>, target: &str, roles: &[&str]) -> Result<i64> {
        let node = self.refs.resolve(target, roles).map_err(Error::Tool)?;
        let valid = tab.zip(self.scope.as_ref()).is_some_and(|(tab, scope)| {
            tab.cdp.alive()
                && scope.cdp.ptr_eq(&Rc::downgrade(&tab.cdp))
                && scope.session == tab.session
                && self.refs.frame_for(node).and_then(|frame| scope.frames.get(frame)).is_some_and(|path| {
                    path.iter().all(|(frame, epoch)| {
                        tab.cdp.events().frame_document(&tab.session, frame) == Some(*epoch)
                    })
                })
        });
        if !valid {
            return Err(Error::Tool("ref has no current document binding; take a fresh snapshot".into()));
        }
        Ok(node)
    }

    pub(crate) fn forget(&mut self) {
        self.discard_refs();
        self.world.reset();
        self.caller_world.reset();
        // A new document is the usual proof that a submission went through,
        // and the point at which sending it again is an ordinary act.
        self.submitted = None;
        // Nothing a new page says is news against what the last one said.
        self.said.clear();
    }

    pub(crate) fn discard_refs(&mut self) {
        self.refs.forget();
        self.scope = None;
        self.signature = None;
        self.printed = None;
    }

    /// The lines of page text that were not there when this session last
    /// acted, capped. Nothing on the first look at a document: everything is
    /// new there, and saying so would say nothing.
    pub(crate) async fn new_text(&mut self, tab: &Tab, limit: usize) -> Option<String> {
        let reading =
            crate::page::reading::read(&mut self.world, tab, false, None, false, None).await.ok()?;
        let mut now = std::collections::HashSet::new();
        let mut fresh: Vec<String> = Vec::new();
        for source in &reading.sources {
            for line in source.text.iter().flat_map(|text| text.lines()) {
                let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
                if line.is_empty() || !now.insert(line.clone()) {
                    continue;
                }
                if !self.said.contains(&line) {
                    fresh.push(format!("\n  {line}"));
                }
            }
        }
        let first = self.said.is_empty();
        self.said = now;
        if first || fresh.is_empty() {
            return None;
        }
        Some(listed("The page now says:", &fresh, limit, |more| format!("\n  … and {more} more new lines")))
    }
}
