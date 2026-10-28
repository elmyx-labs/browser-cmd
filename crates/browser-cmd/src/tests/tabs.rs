//! Tabs on a real browser: windows a click opens, switching and listing,
//! attachments released with their last owner, pages that concurrent
//! sessions never share, and a tab recovered only when it is really gone.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_follows_the_window_a_click_opens_past_other_tabs() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>opener</title>
        <a href="/second" target="_blank">Open second</a>
        <button onclick="window.open('/third', '_blank')">Pop third</button>
        <h1>Opener</h1>"#
            .to_owned(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let host = Host::new(settings);
            // A tab from another session, open the whole time, that this
            // click did not open and must not be mistaken for its window.
            let mut other = host.session();
            let _ = other.execute("navigate", json!({ "url": format!("http://{address}/other") })).await;

            let mut session = host.session();
            let opened =
                session.execute("navigate", json!({ "url": format!("http://{address}/first") })).await;
            assert!(opened.ok, "{}", opened.output);

            let link = session.execute("click", json!({ "ref": "Open second" })).await;
            assert!(link.ok, "{}", link.output);
            assert!(link.output.contains("/second"), "the opened window was not followed: {}", link.output);
            let here = session.execute("eval", json!({ "expression": "location.pathname" })).await;
            assert!(here.output.contains("/second"), "{}", here.output);

            // Back to the opener, then the window.open button.
            let back = session.execute("navigate", json!({ "url": format!("http://{address}/first") })).await;
            assert!(back.ok, "{}", back.output);
            let pop = session.execute("click", json!({ "ref": "Pop third" })).await;
            assert!(pop.ok, "{}", pop.output);
            assert!(pop.output.contains("/third"), "window.open was not followed: {}", pop.output);

            let _ = session.execute("close", json!({})).await;
            let _ = other.execute("close", json!({})).await;
        })
        .await;
}

/// The about:blank from launch is adopted as the first tab, not listed beside it.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_works_across_tabs_without_a_phantom_blank() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <button onclick="document.title='clicked'+location.search">Act</button>"#
            .to_owned(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let a = format!("http://{address}/?p=A");
    let b = format!("http://{address}/?p=B");
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut session = Host::new(settings).session();
            let opened = session.execute("navigate", json!({ "url": a })).await;
            let act =
                opened.output.split(" @").nth(1).and_then(|rest| rest.split_whitespace().next()).unwrap();
            let act = format!("@{act}");
            session.execute("tab", json!({ "action": "new", "url": b })).await;
            let list = session.execute("tab", json!({ "action": "list" })).await;
            assert!(
                list.output.contains("?p=A") && list.output.contains("?p=B"),
                "both tabs should be listed: {}",
                list.output
            );
            assert!(
                !list.output.contains("about:blank"),
                "a phantom blank tab is in the list: {}",
                list.output
            );
            // Back on A, its controls are not new to the agent, and the ref held from it still acts.
            let id = list.output.lines().find(|line| line.contains("?p=A")).unwrap();
            let id = id.trim_start_matches(['*', ' ']).split(' ').next().unwrap();
            assert_eq!(id, "t1", "{}", list.output);
            let back = session.execute("tab", json!({ "action": "switch", "id": id })).await;
            assert!(back.ok && !back.output.contains("[new]"), "{}", back.output);
            let click = session.execute("click", json!({ "ref": act })).await;
            assert!(click.ok, "click after tab work failed: {}", click.output);
            let _ = session.execute("close", json!({})).await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_clicks_land_after_a_session_of_navigations() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <form action="/next"><input aria-label="Name"><button>Go</button></form>
        <button id="act" onclick="document.querySelector('#out').textContent='hit'">Act</button>
        <p id="out">idle</p>"#
            .to_owned(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let host = Host::new(settings);
            // Orphan tabs from sessions that ended without close(): the
            // realistic state of a long-lived served browser.
            for _ in 0..3 {
                let mut ghost = host.session();
                let _ = ghost.execute("navigate", json!({ "url": format!("http://{address}/ghost") })).await;
                std::mem::drop(ghost);
            }
            let mut session = host.session();
            for round in 0..3 {
                let _ = session.execute("close", json!({})).await;
                let one =
                    session.execute("navigate", json!({ "url": format!("http://{address}/login") })).await;
                assert!(one.ok, "{}", one.output);
                let filled =
                    session.execute("type", json!({ "ref": "Name", "text": "x", "submit": true })).await;
                assert!(filled.ok, "{}", filled.output);
                let three =
                    session.execute("navigate", json!({ "url": format!("http://{address}/alerts") })).await;
                assert!(three.ok, "{}", three.output);
                let clicked = session.execute("click", json!({ "ref": "Act" })).await;
                assert!(clicked.ok, "{}", clicked.output);
                let read = session
                    .execute("eval", json!({ "expression": "document.querySelector('#out').textContent" }))
                    .await;
                assert!(
                    read.output.contains("hit"),
                    "round {round}: the click did not land: {}",
                    read.output
                );
            }
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn another_sessions_tab_is_never_adopted_as_a_new_window() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut a = host.session();
            let mut b = host.session();
            let tab = a.current().await.unwrap();
            a.book.before = Some(crate::tabs::page_ids(&a.host).await.unwrap());
            let other = b.current().await.unwrap();
            let (now, notes) = crate::tabs::follow(&mut a, &tab).await.unwrap();
            host.shutdown().await;
            assert!(notes.is_empty() && now.target != other.target, "session A stole session B's tab");
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn attachment_lifetime_follows_last_owner_without_closing_the_page() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let shared = host.shared().await.unwrap();
            let tab = crate::chrome::tab::new_tab(&shared.cdp, None, false).await.unwrap();
            let target = tab.target.clone();
            let session = tab.session.clone();
            let retained = tab.clone();
            drop(tab);
            retained.call("Page.getFrameTree", json!({})).await.unwrap();
            drop(retained);
            let info =
                shared.cdp.call(None, "Target.getTargetInfo", json!({"targetId":target})).await.unwrap();
            assert_eq!(info["targetInfo"]["attached"], false, "last owner leaked its CDP attachment");
            assert!(shared.cdp.call(Some(&session), "Page.getFrameTree", json!({})).await.is_err());
            let reattached = crate::chrome::tab::attach(&shared.cdp, target).await.unwrap();
            reattached.call("Page.getFrameTree", json!({})).await.unwrap();
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn tab_switching_releases_previous_attachments_and_new_never_reuses_blank_pages() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut handler = host.session();
            let initial = handler.current().await.unwrap();
            let first = initial.target.clone();
            let cdp = initial.cdp.clone();
            drop(initial);
            let result = handler.execute("tab", json!({"action":"new"})).await;
            assert!(result.ok, "{}", result.output);
            let second = handler.current().await.unwrap().target.clone();
            assert_ne!(first, second, "tab new reused an existing blank target");
            for target in [&first, &second, &first, &second] {
                let before = handler.current().await.unwrap().session.clone();
                let switched = handler.execute("tab", json!({"action":"switch","id":target})).await;
                assert!(switched.ok, "{}", switched.output);
                assert!(
                    cdp.call(Some(&before), "Page.getFrameTree", json!({})).await.is_err(),
                    "previous attachment survived switching"
                );
                let active = handler.current().await.unwrap().session.clone();
                let repeated = handler.execute("tab", json!({"action":"switch","id":target})).await;
                assert!(repeated.ok, "{}", repeated.output);
                assert_eq!(
                    handler.current().await.unwrap().session,
                    active,
                    "same-target switch created another attachment"
                );
            }
            let before = handler.current().await.unwrap().target.clone();
            let count = crate::tabs::pages(&handler.host).await.unwrap().len();
            let invalid = handler.execute("tab", json!({"action":"new","url":"javascript:alert(1)"})).await;
            assert!(!invalid.ok);
            assert_eq!(handler.current().await.unwrap().target, before);
            assert_eq!(crate::tabs::pages(&handler.host).await.unwrap().len(), count);
            drop(handler);
            let info = cdp.call(None, "Target.getTargetInfo", json!({"targetId":before})).await.unwrap();
            assert_eq!(info["targetInfo"]["attached"], false);
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn concurrent_sessions_claim_distinct_pages_and_do_not_reuse_released_blank_targets() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut a = host.session();
            let mut b = host.session();
            let (one, two) = tokio::join!(a.current(), b.current());
            let one = one.unwrap();
            let two = two.unwrap();
            let old = [one.target.clone(), two.target.clone()];
            assert_ne!(old[0], old[1], "concurrent sessions claimed one blank page");
            drop((one, two, a, b));
            let mut c = host.session();
            let new = c.current().await.unwrap();
            assert!(
                !old.contains(&new.target),
                "a detached blank page was mistaken for an unowned startup page"
            );
            for target in old {
                let info =
                    new.cdp.call(None, "Target.getTargetInfo", json!({"targetId":target})).await.unwrap();
                assert_eq!(info["targetInfo"]["attached"], false);
            }
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn tab_recovery_uses_attachment_events_instead_of_error_words() {
    let address = serve("<!doctype html><title>retained</title><button>Keep me</button>".into());
    on_page(address, async |handler, _| {
        let original = handler.book.active.as_ref().unwrap().clone();
        assert!(original.attached());
        for message in ["closed", "went away", "No target"] {
            let failed = handler
                .execute(
                    "eval",
                    json!({
                        "expression":format!("throw new Error({})", serde_json::to_string(message).unwrap())
                    }),
                )
                .await;
            assert!(!failed.ok, "{}", failed.output);
            assert_eq!(
                handler.book.active.as_ref().expect("error text discarded the tab").session,
                original.session
            );
            let snapshot = handler.execute("snapshot", json!({})).await;
            assert!(snapshot.ok && snapshot.output.contains("Keep me"), "{}", snapshot.output);
            assert_eq!(handler.book.active.as_ref().unwrap().session, original.session);
        }
        // Detaching an observer leaves its target alive. Recover that exact
        // target, and only create a replacement when Chrome confirms it is gone.
        original
            .cdp
            .call(None, "Target.detachFromTarget", json!({"sessionId":original.session}))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while original.attached() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Chrome did not report detachment");
        let restored = handler.current().await.unwrap();
        assert_eq!(restored.target, original.target);
        assert_ne!(restored.session, original.session);
        assert!(restored.attached());
        restored.cdp.close_target(&restored.target).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while restored.attached() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed target stayed attached");
        let replacement = handler.current().await.unwrap();
        assert_ne!(replacement.target, restored.target);
        assert!(replacement.attached());
        assert_eq!(replacement.url().await.unwrap(), "about:blank");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_follows_a_window_a_key_opens_and_returns_when_it_closes() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <a href="/popup" target="_blank" id=a>Open popup</a>
        <button onclick="window.close()">Close me</button>
        <script>onload = () => a.focus()</script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let pressed = session.execute("press", json!({ "key": "Enter" })).await;
        assert!(pressed.ok, "{}", pressed.output);
        assert!(
            pressed.output.contains("A new tab t2 opened at") && pressed.output.contains("/popup"),
            "{}",
            pressed.output
        );
        let closed = session.execute("click", json!({ "ref": "Close me" })).await;
        assert!(closed.ok, "{}", closed.output);
        assert!(closed.output.contains("This tab closed; switched to"), "{}", closed.output);
        let here = session.execute("eval", json!({ "expression": "location.pathname" })).await;
        assert!(here.output.contains("\"/\""), "{}", here.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_closing_the_active_tab_switches_to_the_one_left_last() {
    let address = serve("<!doctype html><title>t</title><p>page".to_owned());
    on_page(address, async |session, _| {
        for path in ["b", "c"] {
            let url = format!("http://{address}/{path}");
            assert!(session.execute("tab", json!({ "action": "new", "url": url })).await.ok);
        }
        for id in ["t1", "t3"] {
            assert!(session.execute("tab", json!({ "action": "switch", "id": id })).await.ok);
        }
        let id = session.book.active.as_ref().unwrap().target.clone();
        let closed = session.execute("tab", json!({ "action": "close", "id": id })).await;
        let back = format!("Closed the tab; switched to http://{address}/.");
        assert_eq!(closed.output, back, "not t2, which was left before t1");
        let active = session.book.active.as_ref().unwrap().target.clone();
        assert!(!session.book.previous.contains(&active), "{:?}", session.book.previous);
        let here = session.execute("eval", json!({ "expression": "location.pathname" })).await;
        assert!(here.output.contains("\"/\""), "{}", here.output);
    })
    .await;
}

/// A tab that cannot be prepared is neither used nor left held: the blank page
/// the launch opened goes back to nobody, and one opened for the session is closed.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_tab_that_cannot_be_prepared_is_not_left_held() {
    let address = serve("<!doctype html><title>t</title>".into());
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path(), temp.path());
    std::fs::write(temp.path().join("file"), "").unwrap();
    settings.downloads = temp.path().join("file").join("downloads");
    let host = Host::new(settings);
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut session = host.session();
            for _ in 0..2 {
                let failed = session.execute("navigate", json!({ "url": format!("http://{address}") })).await;
                assert!(!failed.ok, "{}", failed.output);
            }
            let pages = crate::tabs::pages(&host).await.unwrap();
            assert_eq!(pages.len(), 1, "only the launch's blank page is open: {pages:?}");
            let class = crate::tabs::ledger::of(&host).borrow().class(crate::tabs::target_id(&pages[0]), "");
            assert_eq!(class, crate::tabs::ledger::Class::Unowned);
            host.shutdown().await;
        })
        .await;
}

/// A session ended while another one is starting or reconnecting the browser
/// waits for it, and still closes its tabs, rather than finding none.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_session_ended_while_the_browser_is_busy_for_another_still_closes_its_tabs() {
    on_page(serve("<!doctype html><title>t</title>".into()), async |session, _| {
        let target = session.book.active.as_ref().unwrap().target.clone();
        let host = session.host.clone();
        let held = host.shared.lock().await;
        let release = async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            drop(held);
        };
        let (closed, ()) = tokio::join!(session.execute("close", json!({})), release);
        assert!(closed.output.starts_with("Closed t1."), "{}", closed.output);
        assert!(!host.shared().await.unwrap().cdp.target_exists(&target).await.unwrap());
    })
    .await;
}
