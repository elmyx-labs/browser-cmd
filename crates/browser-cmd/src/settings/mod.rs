//! What the browser can be told. Every field has a default; the few an operator
//! turns are read from `BROWSER_CMD_*` variables, and the rest are constants a
//! test or an embedding application may still set directly.

use std::{path::PathBuf, time::Duration};

pub(crate) mod upstream;

use crate::env::{display_available, flag, parse, var};
use crate::{Error, Result};
use upstream::UpstreamProxy;

/// What to answer when a page opens a modal dialog. Something must be sent at
/// once: a dialog left open blocks the page and every command after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialogs {
    /// Accept what only tells you something, dismiss what asks: answering yes
    /// to a question nobody was asked is a decision made on the caller's behalf.
    AlertsOnly,
    Accept,
    Dismiss,
}

impl Dialogs {
    /// Whether a dialog of this kind (alert, confirm, prompt, beforeunload) is accepted.
    pub fn accepts(self, kind: &str) -> bool {
        match self {
            Self::Accept => true,
            Self::Dismiss => false,
            Self::AlertsOnly => kind == "alert",
        }
    }
}

/// The bounds a page is held to, in bytes unless said otherwise.
#[derive(Clone, Debug)]
pub struct Limits {
    /// How much page text or snapshot one result may carry.
    pub page_text: usize,
    /// How much `text` shows unless asked for another amount; the whole
    /// reading is saved to a file when it is longer.
    pub text: usize,
    /// How much text around each match a `find` shows.
    pub passage: usize,
    /// How much of the page's state every action reports back.
    pub state: usize,
    /// How many images are listed or described.
    pub images: usize,
    /// How many rows one extraction may carry back.
    pub extract_rows: usize,
    /// Fraction of the list viewport revisited on successive extraction passes.
    pub extract_scroll_overlap: f64,
    /// Unchanged observations before extraction reports a stall (not completion).
    pub extract_stall_passes: u32,
    /// How much of one element's markup `markup` shows.
    pub markup: usize,
    /// How much of what the page newly says an action reports back.
    pub new_text: usize,
    /// The largest document the browser will carry out of a viewer.
    pub document: usize,
    /// Pixels: the largest single scroll, and the default one.
    pub scroll_max: u32,
    pub scroll_default: u32,
    /// How many wheel passes a scroll to an end may make before giving up.
    pub scroll_passes: u32,
    /// How many numbered files one directory may hold.
    pub screenshots: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            page_text: 64 * 1024,
            text: 16 * 1024,
            passage: 300,
            state: 12 * 1024,
            images: 200,
            extract_rows: 200,
            extract_scroll_overlap: 0.15,
            extract_stall_passes: 2,
            markup: 8 * 1024,
            new_text: 1024,
            document: 24 * 1024 * 1024,
            scroll_max: 5000,
            scroll_default: 700,
            scroll_passes: 60,
            screenshots: 10_000,
        }
    }
}

/// How long to wait, for what.
#[derive(Clone, Debug)]
pub struct Waits {
    pub launch: Duration,
    /// One DevTools call.
    pub request: Duration,
    /// How long a site gets to start answering a navigation before loading is stopped.
    pub navigate: Duration,
    /// Ceiling on waiting for a page to load and its DOM to go quiet.
    pub settle: Duration,
    /// How long after a requested navigation the page still counts as loading.
    pub quiet: Duration,
    pub poll: Duration,
    /// Default for the `wait` command.
    pub wait: Duration,
    /// How long a challenge gets to let the page through on its own before its box is ticked.
    pub challenge_auto: Duration,
    /// How long to wait for the page after the box was ticked, or for a token challenge.
    pub challenge: Duration,
    pub challenge_poll: Duration,
    /// How long a CAPTCHA widget that draws itself after the page went quiet is given.
    pub widget: Duration,
    pub close: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        let seconds = Duration::from_secs_f64;
        Self {
            launch: seconds(20.0),
            request: seconds(20.0),
            navigate: seconds(10.0),
            settle: seconds(4.0),
            quiet: seconds(0.4),
            poll: seconds(0.1),
            wait: seconds(30.0),
            challenge_auto: seconds(4.0),
            challenge: seconds(20.0),
            challenge_poll: seconds(0.5),
            widget: seconds(3.0),
            close: seconds(5.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Files the model names resolve against; screenshots and downloads land under it.
    pub workspace: PathBuf,
    pub screenshots: PathBuf,
    pub downloads: PathBuf,
    /// Chrome's user data directory: the identity a site remembers.
    pub profile: PathBuf,
    /// A running Chrome's DevTools endpoint to attach to instead of launching
    /// one. Nothing is launched, overridden or closed.
    pub attach: Option<String>,
    pub executable: Option<PathBuf>,
    pub headless: bool,
    pub no_sandbox: bool,
    /// Extra Chrome flags, on top of the normal command line.
    pub chrome_args: Vec<String>,
    pub proxy: Option<UpstreamProxy>,
    /// Hosts, with their subdomains, that go through the proxy; empty sends
    /// everything. A metered proxy spent only where the network matters.
    pub proxy_hosts: Vec<String>,
    /// The headless window, in CSS pixels.
    pub window: (u32, u32),
    /// Accept-Language, as bare comma-separated tags.
    pub language: String,
    /// The IANA zone Chrome runs in, for when the network it browses from is
    /// not where this machine is.
    pub timezone: Option<String>,
    pub dialogs: Dialogs,
    /// Multiplier on every pause of a person's tempo: 1 is a person, 0 is none,
    /// and with none the pointer does not wander either.
    pub pace: f64,
    /// Report the requests a page made and did not get. Off by default: it takes
    /// `Network.enable`, one more thing a wall can look at.
    pub network_notes: bool,
    /// Keep every tab's data responses from its first request, for `network`;
    /// otherwise a tab keeps them from its first `network` on. Also `Network.enable`.
    pub network: bool,
    /// Record the text 2D canvases draw, for the agent to be told. Off by
    /// default: it wraps the page's own canvas functions, which a page can see.
    pub canvas_text: bool,
    /// Tell the agent when it is repeating itself or getting nowhere.
    pub awareness: bool,
    pub limits: Limits,
    pub waits: Waits,
}

impl Settings {
    /// The defaults, for a browser whose home is `home` working in `workspace`.
    pub fn new(home: &std::path::Path, workspace: &std::path::Path) -> Self {
        let screenshots = workspace.join("media/browser");
        Self {
            workspace: workspace.to_owned(),
            downloads: screenshots.join("downloads"),
            screenshots,
            profile: home.join("browser/default"),
            attach: None,
            executable: None,
            headless: !display_available(),
            no_sandbox: false,
            chrome_args: Vec::new(),
            proxy: None,
            proxy_hosts: Vec::new(),
            window: (1440, 900),
            language: "en-US,en".into(),
            timezone: None,
            dialogs: Dialogs::AlertsOnly,
            pace: 1.0,
            network_notes: false,
            network: false,
            canvas_text: false,
            awareness: true,
            limits: Limits::default(),
            waits: Waits::default(),
        }
    }

    /// The defaults, then whatever the environment says.
    pub fn from_env(home: &std::path::Path, workspace: &std::path::Path) -> Result<Self> {
        let mut settings = Self::new(home, &workspace.canonicalize()?);
        if let Some(profile) = var("PROFILE") {
            settings.profile = profile.into();
        }
        settings.attach = var("CDP");
        settings.executable = var("EXECUTABLE").map(PathBuf::from);
        settings.headless = flag("HEADLESS")?.unwrap_or(settings.headless);
        settings.no_sandbox = flag("NO_SANDBOX")?.unwrap_or(false);
        settings.chrome_args =
            var("ARGS").map(|args| args.split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
        settings.proxy = var("PROXY").map(|value| UpstreamProxy::parse(&value)).transpose()?;
        settings.proxy_hosts = var("PROXY_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(|host| host.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect();
        if !settings.proxy_hosts.is_empty() && settings.proxy.is_none() {
            return Err(Error::Config("BROWSER_CMD_PROXY_HOSTS needs BROWSER_CMD_PROXY".into()));
        }
        if let Some(host) = settings
            .proxy_hosts
            .iter()
            .find(|host| !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        {
            return Err(Error::Config(format!("BROWSER_CMD_PROXY_HOSTS entry {host:?} is not a host name")));
        }
        if let Some(window) = var("WINDOW") {
            settings.window = window
                .split_once(',')
                .and_then(|(width, height)| Some((width.trim().parse().ok()?, height.trim().parse().ok()?)))
                .filter(|(width, height)| *width > 0 && *height > 0)
                .ok_or_else(|| Error::Config(format!("BROWSER_CMD_WINDOW={window:?} is not WIDTH,HEIGHT")))?;
        }
        if let Some(language) = var("LANGUAGE") {
            let tags: Vec<&str> = language.split(',').map(str::trim).collect();
            if tags.iter().any(|tag| {
                tag.split('-').any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric()))
            }) {
                return Err(Error::Config(
                    "BROWSER_CMD_LANGUAGE must be bare comma-separated language tags".into(),
                ));
            }
            settings.language = tags.join(",");
        }
        settings.timezone = var("TIMEZONE");
        settings.dialogs = match var("DIALOGS").as_deref() {
            None | Some("alerts") => Dialogs::AlertsOnly,
            Some("accept") => Dialogs::Accept,
            Some("dismiss") => Dialogs::Dismiss,
            Some(other) => {
                return Err(Error::Config(format!(
                    "BROWSER_CMD_DIALOGS={other:?} is not alerts, accept or dismiss"
                )));
            }
        };
        if let Some(pace) = parse::<f64>("PACE")? {
            if !pace.is_finite() || !(0.0..=100.0).contains(&pace) {
                return Err(Error::Config("BROWSER_CMD_PACE must be a number from 0 to 100".into()));
            }
            settings.pace = pace;
        }
        settings.network_notes = flag("NETWORK_NOTES")?.unwrap_or(false);
        settings.network = flag("NETWORK")?.unwrap_or(false);
        settings.canvas_text = flag("CANVAS_TEXT")?.unwrap_or(false);
        settings.awareness = flag("AWARENESS")?.unwrap_or(true);
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_not_answered_for_the_caller() {
        let dialogs = Dialogs::AlertsOnly;
        assert!(dialogs.accepts("alert"));
        assert!(!dialogs.accepts("confirm"));
        assert!(!dialogs.accepts("beforeunload"));
    }
}
