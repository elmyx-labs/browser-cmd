//! The crate's own error, so nothing outside it has to be linked to use it.

use std::{error::Error as StdError, fmt};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What can go wrong driving a browser: a misconfiguration the operator can
/// fix, an I/O or protocol failure, or a tool that could not do what was asked.
#[derive(Debug)]
pub enum Error {
    Config(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Tool(String),
    /// The browser refused a command, saying why.
    Cdp(String),
    /// The connection to the browser is gone.
    Closed,
    Protocol(String),
    DocumentChanged,
    Timeout(&'static str),
    Limit {
        part: &'static str,
        limit: usize,
    },
    /// The browser was told, and never answered. Not a failure: the thing may
    /// have happened. A caller that treats this as "nothing happened" and
    /// tries again is how one order becomes two.
    Unacknowledged(String),
}

impl Error {
    /// The kind of failure, as one word a caller can branch on. The message is
    /// for a reader; this is for the code that must know whether the browser
    /// may have done the thing anyway.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::Tool(_) | Self::Cdp(_) | Self::Closed => "tool",
            Self::Protocol(_) => "protocol",
            Self::DocumentChanged => "document-changed",
            Self::Timeout(_) => "timeout",
            Self::Limit { .. } => "limit",
            Self::Unacknowledged(_) => "unacknowledged",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(value) => write!(f, "configuration error: {value}"),
            Self::Io(value) => write!(f, "io error: {value}"),
            Self::Json(value) => write!(f, "json error: {value}"),
            Self::Tool(value) => write!(f, "tool error: {value}"),
            Self::Cdp(message) => write!(f, "tool error: browser CDP: {message}"),
            Self::Closed => write!(f, "tool error: the browser connection is closed"),
            Self::DocumentChanged => write!(f, "document changed during capture; take a fresh observation"),
            Self::Protocol(value) => write!(f, "protocol error: {value}"),
            Self::Timeout(operation) => write!(f, "{operation} timed out"),
            Self::Limit { part, limit } => write!(f, "{part} exceeds the {limit}-byte limit"),
            Self::Unacknowledged(method) => write!(
                f,
                "outcome unknown: {method} was sent to the browser and never answered. If this was an \
                 action it may have taken effect — look at the page before repeating it"
            ),
        }
    }
}

impl StdError for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unanswered_command_is_named_as_such_and_never_as_a_refusal() {
        assert_eq!(Error::Unacknowledged("Input.dispatchMouseEvent".into()).kind(), "unacknowledged");
        assert_eq!(Error::Tool("no".into()).kind(), "tool");
        let refused = Error::Cdp("No node".into());
        assert_eq!(
            (refused.kind(), refused.to_string().as_str()),
            ("tool", "tool error: browser CDP: No node")
        );
        assert_eq!(Error::Closed.to_string(), "tool error: the browser connection is closed");
        assert_eq!(Error::Timeout("x").kind(), "timeout");
    }
}
