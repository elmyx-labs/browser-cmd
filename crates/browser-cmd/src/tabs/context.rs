//! The browser contexts a session owns after clean retries: the clean retry
//! itself, the current context kept across a replaced browser, the earlier
//! ones disposed once no tab lives there.

use std::rc::Rc;

use crate::browser::host::Shared;
use crate::chrome::cdp::Cdp;
use crate::chrome::tab::Tab;
use crate::page::navigation::{Landing, navigate};
use crate::tabs::book::Book;
use crate::tabs::ledger::Hold;
use crate::tabs::registry::Entry;
use crate::tabs::{Holder, cookies, create, replace};
use crate::web::{web_url, without_challenge};
use crate::{Error, Result};

/// An isolated context belongs to the connection that created or rediscovered it.
#[derive(Clone)]
pub(crate) struct BrowserContext {
    pub(crate) cdp: Rc<Cdp>,
    id: String,
}

impl BrowserContext {
    pub(crate) fn new(cdp: Rc<Cdp>, id: String) -> Self {
        Self { cdp, id }
    }
}

impl std::ops::Deref for BrowserContext {
    type Target = str;
    fn deref(&self) -> &str {
        &self.id
    }
}

impl Book {
    /// Take the contexts of this session's own to the browser's connection
    /// before naming one to it: each the browser still has, rebound; an
    /// earlier one it no longer has, forgotten; the current one, replaced by a
    /// new empty one, which is said.
    pub(crate) async fn reconcile(&mut self, shared: &Rc<Shared>) -> Result<()> {
        self.rebind(shared).await?;
        let Some(context) = &self.context else { return Ok(()) };
        if let Some(context) = rebound(shared, context).await? {
            self.context = Some(context);
            return Ok(());
        }
        let id = shared
            .cdp
            .create_browser_context()
            .await?
            .ok_or_else(|| Error::Protocol("replacement browser returned no context identity".into()))?;
        self.context = Some(BrowserContext::new(shared.cdp.clone(), id));
        Err(Error::Tool(
            "previous isolated context is unavailable; a new empty isolated context was created. Prior \
             in-memory sign-in state was not restored. Take a fresh observation or navigate explicitly"
                .into(),
        ))
    }

    /// The earlier contexts of this session's own, on the browser's connection
    /// now, let go of once no tab of it lives there.
    pub(crate) async fn prune_contexts(&mut self, shared: &Rc<Shared>) -> Result<()> {
        self.rebind(shared).await?;
        self.dispose_contexts(false, |_| true).await.map(drop)
    }

    /// The earlier contexts the browser still has, on its connection; the others forgotten.
    async fn rebind(&mut self, shared: &Rc<Shared>) -> Result<()> {
        let mut at = 0;
        while let Some(context) = self.retired.get(at) {
            match rebound(shared, context).await? {
                Some(context) => (self.retired[at], at) = (context, at + 1),
                None => _ = self.retired.remove(at),
            }
        }
        Ok(())
    }

    /// Dispose the contexts of this session's own — the current one too when
    /// `current` — in which no tab of it that `stays` lives; whether any was.
    /// One whose connection is gone waits for [`Book::reconcile`] to find it
    /// again or forget it.
    pub(crate) async fn dispose_contexts(
        &mut self,
        current: bool,
        stays: impl Fn(&Entry) -> bool,
    ) -> Result<bool> {
        let mut disposed = false;
        while let Some(at) = self.retired.iter().position(|context| self.disposable(context, &stays)) {
            self.retired[at].cdp.dispose_browser_context(&self.retired[at]).await?;
            self.retired.remove(at);
            disposed = true;
        }
        if let Some(context) =
            self.context.as_ref().filter(|context| current && self.disposable(context, &stays))
        {
            context.cdp.dispose_browser_context(context).await?;
            self.context = None;
            disposed = true;
        }
        Ok(disposed)
    }

    fn disposable(&self, context: &BrowserContext, stays: impl Fn(&Entry) -> bool) -> bool {
        context.cdp.alive() && !self.lives(context, stays)
    }

    /// Make a clean context this session's own and the tab retried in it the
    /// one the retry left, which is let go; the context it lived in, when of
    /// this session's own, is disposed once no other tab of the session lives there.
    async fn adopt_clean(&mut self, mut pending: Pending, source: &Tab, tab: &Tab) {
        // Best effort: the retry itself has already succeeded.
        let _ = self.let_go(&pending.cdp, &source.target).await;
        self.retarget(&source.target, &tab.target);
        let clean = BrowserContext::new(pending.cdp.clone(), pending.id.clone());
        self.retired.extend(self.context.replace(clean));
        pending.committed = true;
        let _ = self.dispose_contexts(false, |_| true).await;
    }
}

/// A clean retry: the page `source` shows again, in a new context of this
/// session's own, under the same handle, carrying the site's cookies only
/// when `signin`; the tab it is now and how many cookies came with it.
pub(crate) async fn retry(holder: &mut impl Holder, source: &Tab, signin: bool) -> Result<(Tab, usize)> {
    let info = source.info().await?;
    let url = without_challenge(web_url(&info.url)?);
    let host = holder.host();
    let shared = host.shared().await?;
    // What poisons a page is usually the cache or a reputation, not the
    // sign-in — and signing in again may not be something the agent can
    // do. Carry this site's cookies across when asked, and nothing else.
    let carried = match signin {
        true => cookies::of(&shared.cdp, &url, info.context.as_deref()).await?,
        false => Vec::new(),
    };
    let pending = Pending::create(&shared.cdp).await?;
    cookies::restore(&shared.cdp, &pending.id, &carried).await?;
    let book = holder.book();
    let hold = Hold::Owned(book.name.clone());
    let tab = create(&shared, host.settings(), book, Some(&pending.id), hold).await?;
    let wait = host.settings().waits.navigate;
    match navigate(&tab, url.as_str(), wait).await? {
        Landing::Committed => {}
        Landing::Failed(reason) => {
            return Err(Error::Tool(format!("clean-context navigation failed: {reason}")));
        }
        Landing::NoAnswer => {
            return Err(Error::Tool(format!("{url} did not start answering within {} s", wait.as_secs())));
        }
    }
    holder.book().adopt_clean(pending, source, &tab).await;
    replace(holder, tab.clone());
    Ok((tab, carried.len()))
}

/// A context of this session's own on the browser's connection, when that browser has it.
async fn rebound(shared: &Shared, context: &BrowserContext) -> Result<Option<BrowserContext>> {
    if Rc::ptr_eq(&context.cdp, &shared.cdp) {
        return Ok(Some(context.clone()));
    }
    let exists = shared.cdp.context_exists(context).await?;
    Ok(exists.then(|| BrowserContext::new(shared.cdp.clone(), context.to_string())))
}

/// A replacement context is provisional until preparation succeeds. Failure
/// leaves the old handler identity intact and queues cleanup of the candidate.
struct Pending {
    cdp: Rc<Cdp>,
    id: String,
    committed: bool,
}

impl Pending {
    async fn create(cdp: &Rc<Cdp>) -> Result<Self> {
        let id = cdp
            .create_browser_context()
            .await?
            .ok_or_else(|| Error::Protocol("Chrome returned no browser context identity".into()))?;
        Ok(Self { cdp: cdp.clone(), id, committed: false })
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.committed {
            self.cdp.abandon_browser_context(&self.id);
        }
    }
}
