//! Browser contexts on a real browser: a clean retry and the sign-in it may
//! carry, cookies copied with their attributes, and contexts closed, disposed
//! or lost without falling back to the profile.

use super::*;

#[test]
fn a_clean_retry_leaves_the_challenge_attempt_behind() {
    let at = |input: &str| crate::web::without_challenge(url::Url::parse(input).unwrap()).to_string();
    assert_eq!(at("https://site.test/de/?__cf_chl_rt_tk=abc"), "https://site.test/de/");
    assert_eq!(at("https://site.test/s?q=a+b&__cf_chl_tk=x&page=2"), "https://site.test/s?q=a+b&page=2");
    assert_eq!(at("https://site.test/s?q=1#top"), "https://site.test/s?q=1#top");
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_clean_retry_reloads_the_page_the_session_is_on_now() {
    let address = serve("<!doctype html><a href='/next'>Next</a>".into());
    on_page(address, async |session, _| {
        let clicked = session.execute("click", json!({"ref":"Next"})).await;
        assert!(clicked.ok, "{}", clicked.output);
        let result = session.execute("retry-clean", json!({})).await;
        let at = session.current().await.unwrap().url().await.unwrap();
        assert!(result.ok, "{}", result.output);
        assert!(at.ends_with("/next"), "retried stale URL: {at}");
    })
    .await;
}

/// The cache is what usually poisons a page, and signing in again may not
/// be something an agent can do. A clean retry can keep this site's
/// cookies and nothing else.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_can_carry_a_sign_in_through_a_clean_retry() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // One thread per connection: see `serve`.
            std::thread::spawn(move || {
                let mut request = [0_u8; 8192];
                let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
                let asked = String::from_utf8_lossy(&request[..read]).into_owned();
                let signed = asked.contains("who=ada");
                let body = format!(
                    "<!doctype html><title>t</title><h1>{}</h1>",
                    if signed { "Signed in as ada" } else { "Signed out" }
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: who=ada; Path=/\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            });
        }
    });
    on_page(address, async |session, first| {
        assert!(first.output.contains("Signed out"), "{}", first.output);
        let again = session.execute("navigate", json!({ "url": format!("http://{address}") })).await;
        assert!(again.output.contains("Signed in as ada"), "the cookie took: {}", again.output);

        let bare = session.execute("retry-clean", json!({})).await;
        assert!(bare.ok, "{}", bare.output);
        assert!(bare.output.contains("Signed out"), "a clean retry is clean: {}", bare.output);
        let tabs = session.execute("tab", json!({ "action": "list" })).await;
        let open = (tabs.output.matches(" — ").count(), tabs.output.contains("other tab"));
        assert_eq!(open, (1, false), "the tab left behind was closed: {}", tabs.output);

        let kept = session.execute("retry-clean", json!({ "keep_signin": true })).await;
        assert!(kept.ok, "{}", kept.output);
        assert!(kept.output.contains("carrying 1 cookie"), "{}", kept.output);
        assert!(kept.output.contains("Signed in as ada"), "the sign-in came across: {}", kept.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn cookie_transfer_preserves_native_scope_security_and_partition_attributes() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new().run_until(async {
        let shared = host.shared().await.unwrap();
        shared.cdp.call(None, "Storage.setCookies", json!({"cookies":[
            {"name":"host","value":"one","url":"https://www.example.test/","path":"/api","secure":true,"httpOnly":true,"sameSite":"Lax"},
            {"name":"domain","value":"two","domain":".example.test","path":"/api","secure":true,"httpOnly":true},
            {"name":"parent-only","value":"excluded","url":"https://example.test/","secure":true},
            {"name":"__Host-token","value":"three","url":"https://www.example.test/","path":"/","secure":true},
            {"name":"partitioned","value":"four","url":"https://www.example.test/","path":"/","secure":true,"sameSite":"None",
                "partitionKey":{"topLevelSite":"https://example.test","hasCrossSiteAncestor":false}}
        ]})).await.unwrap();
        let original = shared.cdp.call(None, "Storage.getCookies", json!({})).await.unwrap();
        let selected = crate::tabs::cookies::select(&original, &url::Url::parse("https://www.example.test/").unwrap()).unwrap();
        assert_eq!(selected.len(), 4, "native fixture or scope selection changed");
        assert!(!selected.iter().any(|cookie| cookie["name"] == "parent-only"));
        let created = shared.cdp.call(None, "Target.createBrowserContext", json!({})).await.unwrap();
        let context = created["browserContextId"].as_str().unwrap();
        crate::tabs::cookies::restore(&shared.cdp, context, &selected).await.unwrap();
        let copied = shared.cdp.call(None, "Storage.getCookies", json!({"browserContextId":context})).await.unwrap();
        assert_eq!(copied["cookies"].as_array().unwrap().len(), 4);
        host.shutdown().await;
    }).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn clean_retry_copies_cookies_from_the_switched_tab_context() {
    let address = serve("<!doctype html><h1>Context source</h1>".into());
    let url = format!("http://{address}");
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new().run_until(async {
        let shared = host.shared().await.unwrap();
        let mut contexts = Vec::new();
        for value in ["old-owner", "current-tab"] {
            let created = shared.cdp.call(None, "Target.createBrowserContext", json!({})).await.unwrap();
            let context = created["browserContextId"].as_str().unwrap().to_owned();
            shared.cdp.call(None, "Storage.setCookies", json!({"browserContextId":context,"cookies":[{"name":"who","value":value,"url":url}]})).await.unwrap();
            contexts.push(context);
        }
        let mut handler = host.session();
        handler.book.context = Some(crate::tabs::context::BrowserContext::new(shared.cdp.clone(), contexts[0].clone()));
        handler.current().await.unwrap();
        let other = crate::chrome::tab::new_tab(&shared.cdp, Some(&contexts[1]), false).await.unwrap();
        let navigated = other.call("Page.navigate", json!({"url":url})).await.unwrap();
        assert!(navigated.get("errorText").is_none(), "{navigated}");
        let switched = handler.execute("tab", json!({"action":"use","id":other.target})).await;
        assert!(switched.ok, "{}", switched.output);
        assert_eq!(handler.current().await.unwrap().target, other.target, "tab index selected a different target");
        let retried = handler.execute("retry-clean", json!({"keep_signin":true})).await;
        assert!(retried.ok, "{}", retried.output);
        let cookies = shared.cdp.call(None, "Storage.getCookies", json!({"browserContextId":handler.book.context.as_deref()})).await.unwrap();
        let who = cookies["cookies"].as_array().unwrap().iter().find(|cookie| cookie["name"] == "who").unwrap();
        assert_eq!(who["value"], "current-tab");
        host.shutdown().await;
    }).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn closing_a_context_without_a_current_tab_disposes_its_remaining_pages() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let shared = host.shared().await.unwrap();
            let mut ordinary = host.session();
            let ordinary_target = ordinary.current().await.unwrap().target.clone();
            assert!(
                crate::tools::tabs::close(&mut ordinary, json!({})).await.unwrap().contains("Closed t1.")
            );
            assert!(ordinary.book.active.is_none());
            let ordinary_targets = shared.cdp.call(None, "Target.getTargets", json!({})).await.unwrap();
            assert!(
                !ordinary_targets["targetInfos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["targetId"] == ordinary_target)
            );
            let created = shared.cdp.call(None, "Target.createBrowserContext", json!({})).await.unwrap();
            let context = created["browserContextId"].as_str().unwrap().to_owned();
            let page = crate::chrome::tab::new_tab(&shared.cdp, Some(&context), false).await.unwrap();
            let target = page.target.clone();
            drop(page);
            let mut handler = host.session();
            handler.book.context =
                Some(crate::tabs::context::BrowserContext::new(shared.cdp.clone(), context.clone()));
            let closed = crate::tools::tabs::close(&mut handler, json!({})).await.unwrap();
            assert!(closed.contains("Closed this session's browser context"), "{closed}");
            assert!(handler.book.context.is_none());
            let contexts = shared.cdp.call(None, "Target.getBrowserContexts", json!({})).await.unwrap();
            assert!(!contexts["browserContextIds"].as_array().unwrap().contains(&json!(context)));
            let targets = shared.cdp.call(None, "Target.getTargets", json!({})).await.unwrap();
            assert!(
                !targets["targetInfos"].as_array().unwrap().iter().any(|item| item["targetId"] == target)
            );
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn cleanup_reconciles_externally_closed_tabs_and_disposed_contexts() {
    let address = serve("<!doctype html><h1>Cleanup</h1>".into());
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut handler = host.session();
            let tab = handler.current().await.unwrap();
            let cdp = tab.cdp.clone();
            cdp.call(None, "Target.closeTarget", json!({"targetId":tab.target})).await.unwrap();
            drop(tab);
            assert!(handler.book.active.is_some(), "fixture must leave a stale handler identity");
            assert!(crate::tools::tabs::close(&mut handler, json!({})).await.unwrap().contains("Closed t1."));
            assert!(handler.book.active.is_none());
            let navigated = handler.execute("navigate", json!({"url":format!("http://{address}")})).await;
            assert!(navigated.ok, "{}", navigated.output);
            let clean = handler.execute("retry-clean", json!({})).await;
            assert!(clean.ok, "{}", clean.output);
            let context = handler.book.context.clone().unwrap();
            cdp.call(None, "Target.disposeBrowserContext", json!({"browserContextId":&*context}))
                .await
                .unwrap();
            assert!(handler.book.active.is_some() && handler.book.context.is_some());
            assert!(crate::tools::tabs::close(&mut handler, json!({})).await.unwrap().contains("Closed t2."));
            assert!(handler.book.active.is_none() && handler.book.context.is_none());
            assert!(
                Rc::ptr_eq(&cdp, &host.shared().await.unwrap().cdp),
                "cleanup started a replacement browser"
            );
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn isolated_context_loss_after_browser_restart_is_explicit_and_never_falls_back_to_profile() {
    let address = serve("<!doctype html><button>Action</button>".into());
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new().run_until(async {
        let mut handler = host.session();
        let url = format!("http://{address}");
        assert!(handler.execute("navigate", json!({"url":url})).await.ok);
        assert!(handler.execute("retry-clean", json!({})).await.ok);
        let old_context = handler.book.context.as_deref().unwrap().to_owned();
        let old_ref = handler.document.refs.handles()[0].0.clone();
        let old = host.shared().await.unwrap();
        old.cdp.call(None, "Storage.setCookies", json!({"browserContextId":old_context,"cookies":[{"name":"isolated","value":"previous","url":url}]})).await.unwrap();
        host.shutdown().await;
        let error = match handler.current().await { Err(error) => error.to_string(), Ok(_) => panic!("lost isolation state was silently replaced") };
        assert!(error.contains("new empty isolated context") && error.contains("not restored"), "{error}");
        assert_ne!(handler.book.context.as_deref(), Some(old_context.as_str()));
        let fresh = host.shared().await.unwrap();
        assert!(handler.book.live(&fresh).is_none(), "the lost tab is not taken for a new one");
        assert!(!Rc::ptr_eq(&old.cdp, &fresh.cdp));
        let cookies = fresh.cdp.call(None, "Storage.getCookies", json!({"browserContextId":handler.book.context.as_deref()})).await.unwrap();
        assert!(cookies["cookies"].as_array().unwrap().is_empty());
        let resumed = handler.execute("navigate", json!({"url":url})).await;
        assert!(resumed.ok && resumed.output.contains("the tabs open before are gone"), "{}", resumed.output);
        assert!(handler.document.refs.resolve(&old_ref, &[]).is_err(), "an old ref aliased the replacement page");
        host.shutdown().await;
    }).await;
}

/// A clean retry replaces only the tab it retries: a kept sibling in the
/// context of an earlier retry goes on working there, and that context stays
/// until the session ends with no kept tab in it.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_clean_retry_keeps_a_sibling_tab_working_in_the_context_it_lives_in() {
    let address = serve("<!doctype html><title>t</title><h1>Sibling page</h1>".into());
    on_page(address, async |session, _| {
        assert!(session.execute("retry-clean", json!({})).await.ok);
        let first = session.book.context.as_deref().unwrap().to_owned();
        let b = json!({ "action": "new", "url": format!("http://{address}/b") });
        assert!(session.execute("tab", b).await.ok);
        assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
        let sibling = session.book.active.as_ref().unwrap().target.clone();
        assert!(session.execute("tab", json!({ "action": "switch", "id": "t1" })).await.ok);
        let retried = session.execute("retry-clean", json!({})).await;
        assert!(retried.ok, "{}", retried.output);
        let second = session.book.context.as_deref().unwrap().to_owned();
        assert_ne!(first, second);
        assert_eq!(
            session.book.retired.iter().map(|context| context.to_string()).collect::<Vec<_>>(),
            [first.as_str()]
        );
        assert!(!session.book.previous.iter().any(|target| session.book.registry.of(target).is_none()));
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(
            listed.contains(&format!("*t1 ok http://{address}/")) && listed.contains(" t2 kept ok"),
            "{listed}"
        );
        let read = session.execute("tab", json!({ "action": "switch", "id": "t2" })).await;
        assert!(read.ok && read.output.contains("Switched to t2"), "{}", read.output);
        assert_eq!(session.book.active.as_ref().unwrap().target, sibling);
        let text = session.execute("text", json!({})).await;
        assert!(text.ok && text.output.contains("Sibling page"), "{}", text.output);

        let shared = session.host.shared().await.unwrap();
        let ended = session.execute("close", json!({})).await.output;
        assert_eq!(
            ended,
            "Closed t1. Left kept t2 open. Closed this session's browser context. The session ended; the \
             browser and its profile stay for the next one."
        );
        let contexts = shared.cdp.call(None, "Target.getBrowserContexts", json!({})).await.unwrap();
        let contexts = contexts["browserContextIds"].as_array().unwrap();
        assert!(contexts.contains(&json!(first)) && !contexts.contains(&json!(second)), "{contexts:?}");
        assert!(shared.cdp.target_exists(&sibling).await.unwrap(), "the kept tab was closed");
    })
    .await;
}

/// A site that answered once and then never again does not hold a clean retry.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_bounds_a_clean_retry_the_site_never_answers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 8192];
            let _ = std::io::Read::read(&mut stream, &mut request);
            if !request.starts_with(b"GET / ") {
                continue;
            }
            if held.is_empty() {
                let body = "<!doctype html><title>t</title><h1>Once</h1>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            }
            held.push(stream);
        }
    });
    on_page_with(
        address,
        |settings| settings.waits.navigate = std::time::Duration::from_secs(2),
        async |session, _| {
            let started = std::time::Instant::now();
            let stuck = session.execute("retry-clean", json!({})).await;
            assert!(
                !stuck.ok && stuck.output.contains("did not start answering within 2 s"),
                "{}",
                stuck.output
            );
            assert!(started.elapsed() < std::time::Duration::from_secs(15), "{:?}", started.elapsed());
        },
    )
    .await;
}

/// Ending a session whose browser went, context and all, ends it: there is
/// nothing left to dispose on a connection that is gone.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn closing_a_session_after_its_browser_went_ends_it() {
    on_page(serve("<!doctype html><title>t</title>".into()), async |session, _| {
        assert!(session.execute("retry-clean", json!({})).await.ok);
        session.host.shutdown().await;
        let closed = session.execute("close", json!({})).await;
        assert!(closed.ok && closed.output.contains("The session ended"), "{}", closed.output);
    })
    .await;
}
