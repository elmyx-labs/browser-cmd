//! Starting Chrome the way a person does, so it presents as an ordinary user's browser.
//!
//! Every anti-bot vendor's first questions are answered before a page loads:
//! is this Chrome or Chromium, is the command line an automation bundle, is
//! the window headless, is `navigator.webdriver` set, is the viewport an
//! emulator's default. Real stable Chrome, a normal command line, a real
//! window when the machine has a display, `AutomationControlled` disabled and
//! no viewport emulation answer all of them at once. What remains — the CDP
//! `Runtime.enable` leak — is avoided by never enabling it.
//!
//! Better still is a Chrome nobody automated at all: one a person keeps open,
//! with a debugging port. Attaching to it launches nothing and overrides
//! nothing, which is what keeps the strictest walls calm.

pub(crate) mod arguments;

use std::{net::SocketAddr, path::Path, process::Stdio, time::Duration};

use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::chrome::cdp::Cdp;
use crate::chrome::discover::{active_port, debugger_url, preferred_stable_chrome};
use crate::{Error, Result, Settings};
use arguments::arguments;

/// A browser to drive: attached to, or launched. The child is returned so
/// that dropping it ends what was started here and nothing else.
pub(crate) async fn open(
    settings: &Settings,
    proxy: Option<SocketAddr>,
) -> Result<(Cdp, Option<tokio::process::Child>)> {
    let request = settings.waits.request;
    let dialogs = settings.dialogs;
    match &settings.attach {
        // A ws:// address names one run of Chrome; nothing answers there once it restarts.
        Some(endpoint) if endpoint.starts_with("ws") => {
            let cdp = Cdp::connect(endpoint, request, dialogs).await.map_err(|error| {
                Error::Tool(format!(
                    "the Chrome at the ws:// address in BROWSER_CMD_CDP does not answer ({error}); that \
                     address cannot be reconnected after Chrome restarts, so set BROWSER_CMD_CDP to its \
                     http://HOST:PORT endpoint instead"
                ))
            })?;
            Ok((cdp, None))
        }
        Some(endpoint) => {
            Ok((Cdp::connect(&debugger_url(endpoint, request).await?, request, dialogs).await?, None))
        }
        None => {
            // A Chrome of ours already on this profile — left by a daemon that
            // was killed — holds the profile, so a second one would only exit.
            // It is reused only when it was started exactly as this one would
            // be: one launched without this proxy, or behind a forwarder that
            // died with its daemon, would send every page the wrong way.
            if let Some(url) = active_port(&settings.profile, request).await
                && let Ok(cdp) = Cdp::connect(&url, request, dialogs).await
            {
                let launched = std::fs::read_to_string(settings.profile.join(LAUNCH_RECORD)).ok();
                if proxy.is_none() && launched.as_deref() == Some(launch_record(settings).as_str()) {
                    return Ok((cdp, None));
                }
                retire(&cdp, &settings.profile, settings.waits.close).await?;
            }
            let (child, url) = spawn(settings, proxy).await?;
            Ok((Cdp::connect(&url, request, dialogs).await?, Some(child)))
        }
    }
}

/// Where a launch writes how it started Chrome, beside Chrome's own files.
const LAUNCH_RECORD: &str = "browser-cmd-launch";

/// How Chrome is started, minus the forwarder's port, which no later daemon shares.
fn launch_record(settings: &Settings) -> String {
    let mut record = arguments(settings, None);
    record.push(format!("TZ={}", settings.timezone.as_deref().unwrap_or_default()));
    record.join("\n")
}

/// Close a Chrome left on the profile with other settings, and wait for it to
/// let go of the profile.
async fn retire(cdp: &Cdp, profile: &Path, patience: Duration) -> Result<()> {
    let _ = cdp.call(None, "Browser.close", json!({})).await;
    if released(profile, tokio::time::Instant::now() + patience).await {
        return Ok(());
    }
    Err(Error::Tool(format!(
        "a Chrome started with other settings holds the profile {} and did not close; close it",
        profile.display()
    )))
}

/// Whether the Chrome on a profile let go of it by `deadline`: its lock goes
/// only once the process is gone, after its port closed.
pub(crate) async fn released(profile: &Path, deadline: tokio::time::Instant) -> bool {
    while std::fs::symlink_metadata(profile.join("SingletonLock")).is_ok() {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}

/// Start Chrome and read where it listens off its stderr, which is the one
/// place it says so.
async fn spawn(settings: &Settings, proxy: Option<SocketAddr>) -> Result<(tokio::process::Child, String)> {
    let executable = settings.executable.clone().or_else(preferred_stable_chrome).ok_or_else(|| {
        Error::Config("cannot find Chrome; install it or set BROWSER_CMD_EXECUTABLE".into())
    })?;
    std::fs::create_dir_all(&settings.profile)?;
    owner_only(&settings.profile)?;
    std::fs::write(settings.profile.join(LAUNCH_RECORD), launch_record(settings))?;
    let mut command = tokio::process::Command::new(&executable);
    command.args(arguments(settings, proxy));
    if let Some(zone) = &settings.timezone {
        command.env("TZ", zone);
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| Error::Tool(format!("cannot start Chrome: {error}")))?;
    let stderr = child.stderr.take().ok_or_else(|| Error::Tool("Chrome has no stderr".into()))?;
    // What Chrome said on the way out is the only account of why it left:
    // a profile another Chrome holds, a flag it does not know.
    let listening = async {
        let mut lines = BufReader::new(stderr).lines();
        let mut said = std::collections::VecDeque::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.trim().strip_prefix("DevTools listening on ") {
                return Ok(url.to_owned());
            }
            if said.len() == 5 {
                said.pop_front();
            }
            said.push_back(line);
        }
        Err(said.into_iter().collect::<Vec<_>>().join(" | "))
    };
    match tokio::time::timeout(settings.waits.launch, listening).await {
        Ok(Ok(url)) => Ok((child, url)),
        Ok(Err(said)) => Err(Error::Tool(format!(
            "Chrome exited before opening its DevTools port (is the profile {} open in another \
             Chrome?): {said}",
            settings.profile.display()
        ))),
        Err(_) => Err(Error::Tool("Chrome did not open its DevTools port in time".into())),
    }
}

#[cfg(unix)]
fn owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn owner_only(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(temp: &tempfile::TempDir) -> Settings {
        let mut settings = Settings::new(temp.path(), temp.path());
        settings.headless = true;
        settings
    }

    #[test]
    fn a_launch_is_recorded_without_the_forwarder_port_and_with_what_changes_the_browser() {
        let temp = tempfile::tempdir().unwrap();
        let base = settings(&temp);
        let record = launch_record(&base);
        let mut zoned = base.clone();
        zoned.timezone = Some("America/New_York".into());
        let mut headed = base.clone();
        headed.headless = false;
        assert_ne!(launch_record(&zoned), record);
        assert_ne!(launch_record(&headed), record);
        let proxied = arguments(&base, Some("127.0.0.1:9".parse().unwrap()));
        assert!(proxied.iter().any(|arg| arg.starts_with("--proxy-server=")));
        assert!(!record.contains("--proxy"));
    }

    #[tokio::test]
    async fn a_ws_address_that_no_longer_answers_points_to_the_http_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let mut attached = settings(&temp);
        attached.attach = Some("ws://127.0.0.1:1/devtools/browser/gone".into());
        let error = open(&attached, None).await.err().expect("nothing answers there").to_string();
        assert!(
            error.contains("cannot be reconnected after Chrome restarts") && error.contains("http://"),
            "{error}"
        );
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome; run with --include-ignored"]
    async fn a_leftover_chrome_started_another_way_is_replaced_not_reused() {
        let temp = tempfile::tempdir().unwrap();
        let first = settings(&temp);
        let (mut leftover, _) = spawn(&first, None).await.unwrap();
        // A leftover has long since noted its port; a fresh one notes it a moment after listening.
        while active_port(&first.profile, Duration::from_secs(5)).await.is_none() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let mut second = first.clone();
        second.timezone = Some("America/New_York".into());
        let (cdp, child) = open(&second, None).await.unwrap();
        let mut child = child.expect("a fresh Chrome was launched");
        assert!(
            tokio::time::timeout(Duration::from_secs(10), leftover.wait()).await.is_ok(),
            "the leftover Chrome was closed"
        );
        assert_eq!(
            std::fs::read_to_string(first.profile.join(LAUNCH_RECORD)).unwrap(),
            launch_record(&second)
        );
        drop(cdp);
        child.kill().await.unwrap();
    }
}
