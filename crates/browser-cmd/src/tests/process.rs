//! The browser process: what outlives a run in the profile, a Chrome left by
//! a dead host, and a lost connection reconnected or shut down without a
//! second browser.

use super::*;

/// The browser closes gracefully, so it writes cookies to disk instead of
/// losing them to a hard kill.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_keeps_cookies_across_runs() {
    let address = serve("<!doctype html><meta charset=utf-8><title>t</title><p>hi</p>".to_owned());
    let home = tempfile::tempdir().unwrap();
    let url = format!("http://{address}");
    {
        let settings = settings(home.path(), home.path());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut session = Host::new(settings).session();
                session.execute("navigate", json!({"url":url})).await;
                session
                    .execute("eval", json!({"expression":"document.cookie='auth=1; max-age=100000; path=/'"}))
                    .await;
                // give Chrome a moment to flush
                session.execute("wait", json!({"seconds":1.0})).await;
                let c = session.execute("eval", json!({"expression":"document.cookie"})).await;
                assert!(c.output.contains("auth=1"), "cookie not set in run 1: {}", c.output);
                session.host.shutdown().await;
            })
            .await;
    }
    {
        let settings = settings(home.path(), home.path());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut session = Host::new(settings).session();
                session.execute("navigate", json!({"url":url})).await;
                let c = session.execute("eval", json!({"expression":"document.cookie"})).await;
                assert!(
                    c.output.contains("auth=1"),
                    "the sign-in cookie did not survive the restart: {}",
                    c.output
                );
                session.host.shutdown().await;
            })
            .await;
    }
}

/// localStorage outlasts the run too — many sites keep their auth token there,
/// not in a cookie — because the graceful close flushes it to the profile.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_keeps_localstorage_across_runs() {
    let address = serve("<!doctype html><meta charset=utf-8><title>t</title><p>hi</p>".to_owned());
    let home = tempfile::tempdir().unwrap();
    let url = format!("http://{address}");
    {
        let settings = settings(home.path(), home.path());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut session = Host::new(settings).session();
                session.execute("navigate", json!({"url":url})).await;
                session.execute("eval", json!({"expression":"localStorage.setItem('token','xyz')"})).await;
                session.host.shutdown().await;
            })
            .await;
    }
    {
        let settings = settings(home.path(), home.path());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut session = Host::new(settings).session();
                session.execute("navigate", json!({"url":url})).await;
                let c = session.execute("eval", json!({"expression":"localStorage.getItem('token')"})).await;
                assert!(
                    c.output.contains("xyz"),
                    "the auth token in localStorage did not survive the restart: {}",
                    c.output
                );
                session.host.shutdown().await;
            })
            .await;
    }
}

/// A Chrome left on the profile by a host that died is picked up, and
/// answers, rather than being attached to and then waited on forever.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_left_by_a_dead_host_is_picked_up_and_answers() {
    let address = serve("<!doctype html><title>left</title><h1>Left behind</h1>".to_owned());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let first = Host::new(settings.clone());
            let mut session = first.session();
            let opened = session.execute("navigate", json!({ "url": format!("http://{address}") })).await;
            assert!(opened.ok, "{}", opened.output);
            // The host dies without its Chrome: a kill -9, a crash, a
            // supervisor that reaped the server and not its child.
            std::mem::forget(session);
            std::mem::forget(first);

            let mut again = Host::new(settings).session();
            let answered = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                again.execute("navigate", json!({ "url": format!("http://{address}") })),
            )
            .await
            .expect("the Chrome left behind was attached to and never answered");
            assert!(answered.ok, "{}", answered.output);
            assert!(answered.output.contains("Left behind"), "{}", answered.output);
            // The Chrome it picked up is this crate's, on its profile: a graceful
            // exit closes it rather than leaving it to nobody.
            again.host.shutdown().await;
            let gone = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            let profile = &again.settings().profile;
            assert!(
                crate::chrome::launch::released(profile, gone).await,
                "the picked-up Chrome was left running"
            );
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn isolated_context_and_target_are_rediscovered_on_a_new_connection_to_the_same_browser() {
    for isolated in [false, true] {
        let address = serve("<!doctype html><button>Retained</button>".into());
        let temp = tempfile::tempdir().unwrap();
        let host = Host::new(settings(temp.path(), temp.path()));
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut handler = host.session();
                assert!(handler.execute("navigate", json!({"url":format!("http://{address}")})).await.ok);
                if isolated {
                    assert!(handler.execute("retry-clean", json!({})).await.ok);
                }
                let old_context = handler.book.context.as_deref().map(str::to_owned);
                let previous = handler.current().await.unwrap();
                let target = previous.target.clone();
                handler
                    .document
                    .world
                    .evaluate(&previous, "document.body.dataset.persisted='yes'")
                    .await
                    .unwrap();
                drop(previous);
                let owner = host.shared().await.unwrap();
                let process = Rc::downgrade(owner.process.as_ref().unwrap());
                let pid = owner.process.as_ref().unwrap().chrome.borrow().id();
                let old_cdp = owner.cdp.clone();
                old_cdp.disconnect();
                drop(owner); // Do not keep the old Shared alive and mask ownership bugs.
                let reconnected = host.shared().await.unwrap();
                let cdp = reconnected.cdp.clone();
                assert!(!Rc::ptr_eq(&old_cdp, &cdp));
                assert_eq!(old_cdp.endpoint(), cdp.endpoint());
                assert!(Rc::ptr_eq(&process.upgrade().unwrap(), reconnected.process.as_ref().unwrap()));
                assert_eq!(reconnected.process.as_ref().unwrap().chrome.borrow().id(), pid);
                assert!(reconnected.process.as_ref().unwrap().alive().unwrap());
                let recovered = handler.current().await.unwrap();
                assert_eq!(recovered.target, target);
                assert_eq!(handler.book.context.as_deref(), old_context.as_deref());
                if let Some(context) = &handler.book.context {
                    assert!(Rc::ptr_eq(&context.cdp, &cdp));
                }
                assert_eq!(
                    handler
                        .document
                        .world
                        .evaluate(&recovered, "document.body.dataset.persisted")
                        .await
                        .unwrap(),
                    "yes"
                );
                host.shutdown().await;
                tokio::time::timeout(host.settings().waits.close, async {
                    while reconnected.process.as_ref().unwrap().alive().unwrap() {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("shutdown lost ownership after reconnect");
            })
            .await;
    }
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn shutdown_after_cdp_disconnect_closes_the_owned_process_without_relaunching() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let original = host.shared().await.unwrap();
            original.cdp.disconnect();
            host.shutdown().await;
            assert!(
                !original.process.as_ref().unwrap().alive().unwrap(),
                "shutdown left its Chrome process running"
            );
            assert!(
                Rc::ptr_eq(host.shared.lock().await.as_ref().unwrap(), &original),
                "shutdown replaced the browser instance"
            );
        })
        .await;
}

/// An earlier context of the session's own that still holds one of its tabs
/// is found again on a new connection to the same browser, and disposed once
/// that tab is gone, rather than forgotten and left in the browser.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn an_earlier_own_context_outlives_a_new_connection_and_goes_once_its_tab_does() {
    let address = serve("<!doctype html><title>t</title><h1>Here</h1>".into());
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut session = host.session();
            let url = format!("http://{address}/");
            assert!(session.execute("navigate", json!({ "url": url })).await.ok);
            assert!(session.execute("retry-clean", json!({})).await.ok);
            assert!(session.execute("tab", json!({ "action": "new", "url": url })).await.ok);
            assert!(session.execute("tab", json!({ "action": "switch", "id": "t1" })).await.ok);
            assert!(session.execute("retry-clean", json!({})).await.ok);
            let earlier = session.book.retired.iter().map(|context| context.to_string()).collect::<Vec<_>>();
            assert_eq!(earlier.len(), 1, "t2 holds the first clean context");
            let old = host.shared().await.unwrap();
            old.cdp.disconnect();
            drop(old);
            assert!(session.execute("tab", json!({ "action": "switch", "id": "t2" })).await.ok);
            assert!(session.execute("tab", json!({ "action": "close", "id": "t2" })).await.ok);
            assert!(session.execute("tab", json!({ "action": "list" })).await.ok);
            let shared = host.shared().await.unwrap();
            assert!(!shared.cdp.context_exists(&earlier[0]).await.unwrap(), "the earlier context was left");
            assert!(session.book.retired.is_empty());
            host.shutdown().await;
        })
        .await;
}
