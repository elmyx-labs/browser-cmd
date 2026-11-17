//! The browser a session runs in: the Chrome process this crate owns, the
//! proxy in front of it, and the settings every session reads. One host serves
//! many sessions; the process outlives any one of them.

use std::rc::Rc;

use crate::browser::identity::Identity;
use crate::chrome::cdp::Cdp;
use crate::{Error, Result, Settings};

/// One browser, shared by every session on it.
pub struct Host {
    pub(crate) settings: Settings,
    pub(crate) shared: tokio::sync::Mutex<Option<Rc<Shared>>>,
    /// What a layer above keeps of this browser across its connections, opaque here.
    pub(crate) above: std::cell::OnceCell<Rc<dyn std::any::Any>>,
    /// The profile is this host's while it holds this: see [`Host::own_profile`].
    pub(crate) owned: std::cell::RefCell<Option<std::fs::File>>,
}

pub(crate) struct Shared {
    pub(crate) cdp: Rc<Cdp>,
    pub(crate) identity: Option<Identity>,
    /// Process/proxy ownership survives replacement of a CDP connection.
    pub(crate) process: Option<Rc<BrowserProcess>>,
}

pub(crate) struct BrowserProcess {
    pub(crate) chrome: std::cell::RefCell<tokio::process::Child>,
    pub(crate) proxy: Option<ProxyTask>,
}

impl BrowserProcess {
    pub(crate) fn alive(&self) -> Result<bool> {
        Ok(self.chrome.borrow_mut().try_wait()?.is_none())
    }

    async fn wait_exit(&self, deadline: tokio::time::Instant) {
        while self.alive().unwrap_or(false) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

pub(crate) struct ProxyTask(pub(crate) tokio::task::JoinHandle<()>);
impl Drop for ProxyTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let _ = self.chrome.get_mut().start_kill();
    }
}

impl Host {
    #[must_use]
    pub fn new(settings: Settings) -> Rc<Self> {
        Rc::new(Self {
            settings,
            shared: tokio::sync::Mutex::new(None),
            above: std::cell::OnceCell::new(),
            owned: std::cell::RefCell::new(None),
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Close the browser gracefully so it writes cookies and localStorage to
    /// the profile before it exits — a hard kill loses a fresh sign-in, which
    /// is what makes a session outlast the run. Best-effort: if the browser
    /// does not go on its own, the kill in Drop still ends it. A Chrome on the
    /// profile that an earlier daemon started is closed too; one this host only
    /// attached to is left as it is.
    pub async fn shutdown(&self) {
        let slot = self.shared.lock().await;
        // Whatever happens below, the profile is no longer this host's to hold.
        let _released = self.owned.borrow_mut().take();
        let Some(shared) = slot.as_ref().filter(|_| self.settings.attach.is_none()) else { return };
        let deadline = tokio::time::Instant::now() + self.settings.waits.close;
        let Some(process) = &shared.process else {
            // No child to wait on: the profile's lock says when it went.
            let _ = tokio::time::timeout_at(deadline, self.close(shared)).await;
            crate::chrome::launch::released(&self.settings.profile, deadline).await;
            return;
        };
        if process.alive().unwrap_or(false) {
            let _ = tokio::time::timeout_at(deadline, self.close(shared)).await;
            process.wait_exit(deadline).await;
            // A Chrome that would not close is killed, not left behind: an
            // orphan outlives its profile and crashes when that is removed.
            if process.alive().unwrap_or(false) {
                let _ = process.chrome.borrow_mut().start_kill();
            }
        }
        if !process.alive().unwrap_or(true)
            && let Some(proxy) = &process.proxy
        {
            proxy.0.abort();
        }
    }

    /// One daemon to a profile: a second would share the first one's Chrome,
    /// close it for both on its way out and send its downloads elsewhere. The
    /// lock is the system's, so it goes with the process that held it — a
    /// moment after that process, which is what a daemon just stopped is waited for.
    async fn own_profile(&self) -> Result<()> {
        if self.owned.borrow().is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.settings.profile)?;
        let lock = std::fs::File::create(self.settings.profile.join("browser-cmd-owner"))?;
        let patience = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while lock.try_lock().is_err() {
            if tokio::time::Instant::now() >= patience {
                return Err(Error::Config(format!(
                    "the profile {} is in use by another browser-cmd daemon; give this one its own \
                     BROWSER_CMD_PROFILE, or use that daemon's BROWSER_CMD_ADDR",
                    self.settings.profile.display()
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        *self.owned.borrow_mut() = Some(lock);
        Ok(())
    }

    /// Ask the browser to close, over a new connection to its endpoint when
    /// the last one dropped; cleanup never launches Chrome.
    async fn close(&self, shared: &Shared) -> Result<()> {
        let reconnected;
        let cdp = match shared.cdp.alive() {
            true => &*shared.cdp,
            false => {
                reconnected =
                    Cdp::connect(shared.cdp.endpoint(), self.settings.waits.close, self.settings.dialogs)
                        .await?;
                &reconnected
            }
        };
        cdp.call(None, "Browser.close", serde_json::json!({})).await.map(drop)
    }

    /// The browser while it runs and answers, without starting one; one being
    /// started or reconnected for another session is waited for.
    pub(crate) async fn running(&self) -> Option<Rc<Shared>> {
        self.shared.lock().await.clone().filter(|shared| shared.cdp.alive())
    }

    /// The browser, started on first use and again if it went away. The lock
    /// is what keeps two sessions from starting two Chromes on one profile.
    pub(crate) async fn shared(&self) -> Result<Rc<Shared>> {
        let mut slot = self.shared.lock().await;
        if let Some(shared) = &*slot
            && shared.cdp.alive()
        {
            return Ok(shared.clone());
        }
        if let Some(previous) = &*slot
            && let Some(process) = &previous.process
            && process.alive()?
        {
            // A lost debugging socket is not browser death. Reconnect to this
            // process's exact endpoint and retain its existing proxy and owner.
            // A process that lives but no longer answers at all is hung: it is
            // stopped, never run beside, and the next command starts afresh.
            let cdp = match Cdp::connect(
                previous.cdp.endpoint(),
                self.settings.waits.request,
                self.settings.dialogs,
            )
            .await
            {
                Ok(cdp) => cdp,
                Err(error) => {
                    let _ = process.chrome.borrow_mut().start_kill();
                    process.wait_exit(tokio::time::Instant::now() + self.settings.waits.close).await;
                    return Err(Error::Tool(format!(
                        "the browser stopped answering ({error}) and was stopped; the next command starts a \
                         fresh one, so its tabs and page state are gone: navigate to the page again"
                    )));
                }
            };
            cdp.discover().await?;
            cdp.pace(self.settings.pace);
            if let Some(identity) = &previous.identity {
                cdp.disguise(identity.overrides()).await?;
            }
            let shared = Rc::new(Shared {
                cdp: Rc::new(cdp),
                identity: previous.identity.clone(),
                process: Some(process.clone()),
            });
            *slot = Some(shared.clone());
            return Ok(shared);
        }
        if self.settings.attach.is_none() {
            self.own_profile().await?;
        }
        let guard = match (&self.settings.proxy, &self.settings.attach) {
            (Some(upstream), None) => {
                let (address, task) =
                    crate::chrome::proxy::start(upstream.clone(), Default::default()).await?;
                Some((address, ProxyTask(task)))
            }
            _ => None,
        };
        let proxy = guard.as_ref().map(|(address, _)| *address);
        let (cdp, chrome) = crate::chrome::launch::open(&self.settings, proxy).await?;
        let cdp = Rc::new(cdp);
        let identity = match self.settings.attach.is_none() && self.settings.headless {
            true => Some(crate::browser::identity::identity(&cdp, &self.settings).await?),
            false => None,
        };
        cdp.discover().await?;
        cdp.pace(self.settings.pace);
        if let Some(identity) = &identity {
            cdp.disguise(identity.overrides()).await?;
        }
        let shared = Shared {
            cdp,
            identity,
            process: chrome.map(|chrome| {
                Rc::new(BrowserProcess {
                    chrome: std::cell::RefCell::new(chrome),
                    proxy: guard.map(|(_, task)| task),
                })
            }),
        };
        let shared = Rc::new(shared);
        *slot = Some(shared.clone());
        Ok(shared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_profile_belongs_to_one_host_until_that_host_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let host = || Host::new(Settings::new(temp.path(), temp.path()));
        let (first, second) = (host(), host());
        first.own_profile().await.unwrap();
        first.own_profile().await.unwrap();
        let refused = second.own_profile().await.unwrap_err().to_string();
        assert!(refused.contains("in use by another browser-cmd daemon"), "{refused}");
        first.owned.borrow_mut().take();
        second.own_profile().await.unwrap();
    }
}
