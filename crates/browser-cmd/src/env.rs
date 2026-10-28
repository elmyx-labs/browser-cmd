//! The environment: `BROWSER_CMD_*` variables, read strictly — a value that does
//! not parse is a configuration error, never a silent default — and whether a
//! window can be shown.

use std::time::Duration;

use crate::{Error, Result};

/// The variable's value, when it is set to something other than blanks.
pub(crate) fn var(name: &str) -> Option<String> {
    std::env::var(format!("BROWSER_CMD_{name}"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) fn flag(name: &str) -> Result<Option<bool>> {
    var(name)
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(Error::Config(format!("BROWSER_CMD_{name}={value:?} is not a yes or a no"))),
        })
        .transpose()
}

pub(crate) fn parse<T: std::str::FromStr>(name: &str) -> Result<Option<T>> {
    var(name)
        .map(|value| {
            value.parse().map_err(|_| Error::Config(format!("BROWSER_CMD_{name}={value:?} does not parse")))
        })
        .transpose()
}

/// Positive finite seconds that fit the clock.
pub(crate) fn seconds(name: &str) -> Result<Option<Duration>> {
    parse::<f64>(name)?
        .map(|value| {
            Duration::try_from_secs_f64(value)
                .ok()
                .filter(|duration| {
                    !duration.is_zero() && std::time::Instant::now().checked_add(*duration).is_some()
                })
                .ok_or_else(|| Error::Config(format!("BROWSER_CMD_{name} must be positive finite seconds")))
        })
        .transpose()
}

/// Whether a window can be shown. A real window is the strongest single
/// signal of a real user, and on a desktop it costs nothing.
pub(crate) fn display_available() -> bool {
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}
