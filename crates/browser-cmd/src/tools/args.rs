//! The shapes a command's arguments come in.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Error, Result};

/// Declare a command-local input shape and deserialize it with the shared
/// argument rules. Serde field and container attributes stay at the call site.
macro_rules! args {
    ($value:expr, $name:ident $(, #[$attribute:meta])* { $($fields:tt)* }) => {{
        #[derive(serde::Deserialize)]
        $(#[$attribute])*
        struct $name { $($fields)* }
        $crate::tools::args::parse::<$name>($value)?
    }};
}
pub(crate) use args;

/// A command that takes no arguments, so a mistyped one is refused rather than ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Empty {}

pub(crate) fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
    let args = if args.is_null() { json!({}) } else { args };
    if !args.is_object() {
        return Err(Error::Tool("arguments must be an object".into()));
    }
    serde_json::from_value(args).map_err(|error| Error::Tool(format!("arguments: {error}")))
}

/// A `verify` field that is one phrase or a list of them: each is its own
/// condition to hold, so several constraints check in one call.
#[derive(Default, Deserialize)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub(crate) fn all(&self) -> Vec<&str> {
        match self {
            Self::None => Vec::new(),
            Self::One(one) => vec![one.as_str()],
            Self::Many(many) => many.iter().map(String::as_str).collect(),
        }
    }
}

/// Where an action lands: an element from the latest state, or a point.
#[derive(Default, Deserialize)]
pub(crate) struct Target {
    pub(crate) r#ref: Option<String>,
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
}

impl Target {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(reference) = &self.r#ref {
            if reference.trim().is_empty() {
                return Err(Error::Tool("target ref or name must not be empty".into()));
            }
            return Ok(());
        }
        if self.x.is_some_and(f64::is_finite) && self.y.is_some_and(f64::is_finite) {
            return Ok(());
        }
        Err(Error::Tool("target needs a ref/name or both finite x and y coordinates".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_keep_field_defaults_required_fields_and_unknown_field_policy() -> Result<()> {
        let parsed = args!(json!({ "name": "search" }), Args, #[serde(deny_unknown_fields)] {
            name: String,
            #[serde(default)]
            submit: bool,
        });
        assert_eq!(parsed.name, "search");
        assert!(!parsed.submit);

        let missing_required = (|| -> Result<()> {
            let args = args!(json!({ "submit": true }), Args, #[serde(deny_unknown_fields)] {
                name: String,
                #[serde(default)]
                submit: bool,
            });
            let _ = (&args.name, args.submit);
            Ok(())
        })();
        assert!(missing_required.is_err());

        let unknown = (|| -> Result<()> {
            let args = args!(json!({ "name": "search", "extra": true }), Args, #[serde(deny_unknown_fields)] {
                name: String,
                #[serde(default)]
                submit: bool,
            });
            let _ = (&args.name, args.submit);
            Ok(())
        })();
        assert!(unknown.is_err());
        Ok(())
    }
}
