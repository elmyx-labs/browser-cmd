//! Turning a live page into something an agent can quote: a settled document,
//! an accessibility tree with a ref on everything actionable, the text behind
//! it, and one observation record that says how much of it was captured.

pub(crate) mod canvas;
pub(crate) mod dom;
pub(crate) mod frames;
pub(crate) mod js;
pub(crate) mod navigation;
pub(crate) mod overlay;
pub(crate) mod reading;
pub(crate) mod settle;
pub(crate) mod snapshot;
pub(crate) mod view;
pub(crate) mod walls;
pub(crate) mod world;
