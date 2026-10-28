#![forbid(unsafe_code)]

//! Browser use over Chrome's DevTools Protocol, with nothing between them.
//!
//! Chrome is the only runtime process. This crate owns the connection, the
//! compact accessibility snapshot, the `@e1a.31` handles, and every operation the
//! model is offered. It is built for sites that check whether a person is
//! browsing and for pages that draw themselves after they load, because those
//! are the two things a browser is for that fetching the HTML is not.
//!
//! It runs a real Chrome that presents itself as an ordinary user's browser:
//! stable Google Chrome with a normal command line and no automation flags, a
//! window when there is a display and a headful-equivalent fingerprint when
//! there is not, no `Runtime.enable`, every script in an isolated world, and a
//! pointer and keyboard at a person's tempo. A simple "verify you are human"
//! checkbox is ticked the way a user would tick it.
//!
//! Every action ends the same way: the page settles, a wall is stood at, and
//! the state comes back — where the tab is, what happened meanwhile, and the
//! controls on the page with their handles — so that the next action needs no
//! look first. A handle keeps naming its element for as long as the document
//! lives, and an element can be named by the words on it instead.
//!
//! A `Host` is one browser — launched, or attached to — and a `Session` is one
//! agent on it: one tab, one set of handles. The daemon keeps one host and hands
//! out sessions; the CLI reaches them over loopback HTTP.

mod api;
mod browser;
mod chrome;
mod env;
mod error;
mod http;
mod input;
mod page;
mod session;
mod settings;
mod tabs;
mod text;
mod tools;
mod web;
mod workspace;

pub use api::app::run;

pub(crate) use browser::host::Host;
pub(crate) use error::{Error, Result};
pub(crate) use session::{Image, Outcome};
pub(crate) use settings::Settings;

#[cfg(test)]
mod tests;
