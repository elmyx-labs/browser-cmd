//! The state every action answers with: where the tab is, how far it is
//! scrolled, and the controls on it with the refs that address them.

use std::rc::Rc;

use serde_json::Value;

use crate::Result;
use crate::chrome::tab::Tab;
use crate::page::frames::FrameSet;
use crate::page::snapshot::capture::{Capture, capture};
use crate::session::document::{Document, ReferenceScope};
use crate::web::web_url;

/// The notes a state says in full once per session.
const TIME: &str = "time";
const BUSY: &str = "busy";

pub(crate) struct State {
    url: String,
    /// The address as the browser has it, before it is shortened for showing.
    pub(crate) address: String,
    pub(crate) title: String,
    /// The header after the URL and title, and what the render no longer shows.
    head: String,
    pub(crate) tree: String,
    pub(crate) empty: bool,
    /// What `js::DOCUMENT` read of the page, for the notes an action ends with.
    pub(crate) document: Value,
}

impl Document {
    pub(crate) async fn state(&mut self, tab: &Tab, full: bool, urls: bool) -> Result<State> {
        // A tab attached anew, as one switched back to is, counts its frames'
        // documents afresh; its main document, still the same load, keeps its refs.
        let reattached = self.scope.take().is_some_and(|scope| scope.session != tab.session);
        let captured = FrameSet::capture(tab).await?;
        let tree = &captured.tree;
        // A click that led somewhere else is a navigation too: the handles
        // named nodes of a document that no longer exists.
        let loader = tree["frameTree"]["frame"]["loaderId"].as_str().map(str::to_owned);
        if loader != self.loader {
            self.forget();
            self.loader = loader;
        }
        let Capture { frames, bindings, complete } = capture(tab, tree).await?;
        if reattached && let Some(main) = tree["frameTree"]["frame"]["id"].as_str() {
            self.refs.reattached(main, &bindings);
        }
        self.refs.bind_documents(&bindings);
        let (rendered, empty) = self.refs.render(&frames, full, urls)?;
        // What this render no longer shows, which no line can carry.
        let gone = self.refs.gone().map_or_else(String::new, |gone| format!("{gone}\n\n"));
        let address = tab.url().await?;
        let last_url = web_url(&address).ok();
        let url = crate::web::shown(&address);
        let document =
            self.world.call(tab, crate::page::js::DOCUMENT, Vec::new()).await.unwrap_or(Value::Null);
        // Where in the page this is: the one thing a screen of controls cannot
        // say about itself, and what tells the start of a list from its end.
        let reach = crate::page::view::reach(&mut self.world, tab, None).await;
        let seen = last_url.as_ref().map_or(String::new(), url::Url::to_string);
        self.signature = self.refs.signature(&seen, &reach, complete);
        let place = match (reach["top"].as_f64(), reach["max"].as_f64()) {
            (Some(at), Some(max)) if max > 0.0 => {
                let edge = if at <= 0.0 {
                    " (top)"
                } else if at >= max {
                    " (bottom)"
                } else {
                    ""
                };
                format!("\nScroll: {at:.0} of {max:.0}{edge}")
            }
            _ => String::new(),
        };
        let ready = document["ready"].as_str().unwrap_or("unknown");
        let readiness = if ready != "complete" || document["busy"] == true || empty {
            format!(
                "\nDocument: {ready}; page reports busy: {}.{}",
                document["busy"].as_bool().map_or("unknown", |busy| if busy { "true" } else { "false" }),
                if self.told.contains(BUSY) { "" } else { " This does not establish application readiness." }
            )
        } else {
            String::new()
        };
        let title = document["title"].as_str().unwrap_or_default().to_owned();
        // A task that says "today" or "this weekend" needs the date the
        // browser lives in, which nothing on the page states.
        let clock = match (self.told.contains(TIME), document["clock"].as_str()) {
            (false, Some(clock)) => format!("\nLocal time: {clock}"),
            _ => String::new(),
        };
        // Discard an inconsistent capture without erasing an unrelated
        // main-document submission guard.
        captured.epoch.check(tab).inspect_err(|_| self.discard_refs())?;
        if !clock.is_empty() {
            self.told.insert(TIME);
        }
        if !readiness.is_empty() {
            self.told.insert(BUSY);
        }
        self.scope = Some(ReferenceScope {
            cdp: Rc::downgrade(&tab.cdp),
            session: tab.session.clone(),
            frames: bindings,
        });
        let head = format!("{place}{readiness}{clock}\n\n{gone}");
        Ok(State { url, address, title, head, tree: rendered, empty, document })
    }

    /// A state as the agent is shown it: the title only when it or the address
    /// changed since this tab's last state; then `instead` of the tree, or the
    /// tree, or one line when it is the tree this document last showed whole.
    pub(crate) fn shown(&mut self, state: State, instead: Option<String>) -> String {
        let named = (state.url, state.title);
        let title = match self.named.as_ref() == Some(&named) {
            true => String::new(),
            false => format!("\nTitle: {}", named.1),
        };
        let again =
            format!("{} controls as last shown — `snapshot full` reprints", self.refs.handles().len());
        let tree = match instead {
            Some(instead) => instead,
            None if !state.empty
                && again.len() < state.tree.len()
                && self.printed.as_deref() == Some(state.tree.as_str()) =>
            {
                again
            }
            None => {
                self.printed = Some(state.tree.clone());
                state.tree
            }
        };
        let shown = format!("URL: {}{title}{}{tree}", named.0, state.head);
        self.named = Some(named);
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(title: &str, tree: &str) -> State {
        let (url, title, head) = ("http://a.test/".into(), title.into(), "\n\n".into());
        let address = String::new();
        State { url, address, title, head, tree: tree.into(), empty: false, document: Value::Null }
    }

    #[test]
    fn a_title_and_a_tree_already_shown_whole_are_not_shown_again() {
        let mut document = Document::new();
        let tree = "- button \"Add to basket\" @e1.1\n- button \"Remove from basket\" @e1.2";
        let first = document.shown(state("Shop", tree), None);
        assert_eq!(first, format!("URL: http://a.test/\nTitle: Shop\n\n{tree}"));
        let again = document.shown(state("Shop", tree), None);
        assert_eq!(again, "URL: http://a.test/\n\n0 controls as last shown — `snapshot full` reprints");
        let whole = document.shown(state("Cart", tree), Some(tree.into()));
        assert_eq!(whole, format!("URL: http://a.test/\nTitle: Cart\n\n{tree}"), "a new title is said");
        document.discard_refs();
        assert!(document.shown(state("Cart", tree), None).ends_with(tree), "new refs: the tree again");
    }
}
