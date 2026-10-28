//! Owning a Chrome and speaking to it: finding and launching the process, the
//! wire and the events it volunteers, the tabs, and the route its traffic takes.
//! Everything above this layer assumes a browser that is running and answering.

pub(crate) mod cdp;
pub(crate) mod discover;
pub(crate) mod events;
pub(crate) mod launch;
pub(crate) mod proxy;
pub(crate) mod tab;
pub(crate) mod traffic;
