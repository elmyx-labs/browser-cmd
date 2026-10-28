//! What the CLI sends the daemon; the outcome it answers with is the session's.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
pub(crate) struct Request {
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Value,
}
