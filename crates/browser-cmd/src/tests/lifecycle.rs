//! Tabs and contexts against a scripted browser: what is closed, disposed and
//! carried, and that no cleanup is replayed or aimed at the wrong resource.

use super::*;
use futures_util::{SinkExt, StreamExt};

async fn scripted_host(
    temp: &tempfile::TempDir,
    responses: Vec<(&'static str, Value)>,
) -> (Rc<Host>, Script) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let remaining = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(responses.len()));
    let pending = remaining.clone();
    let worker = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut responses = responses.into_iter();
        while let Some(Ok(message)) = socket.next().await {
            let Ok(text) = message.to_text() else { continue };
            let request: Value = serde_json::from_str(text).unwrap();
            let mut answer = if request["method"] == "Target.detachFromTarget" {
                json!({"result":{}})
            } else {
                let (method, reply) = responses.next().expect("unexpected extra browser operation");
                assert_eq!(request["method"], method, "cleanup replayed or queried the wrong resource");
                pending.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                reply
            };
            if let Some(target) = answer.as_object_mut().and_then(|object| object.remove("_expected_target"))
            {
                assert_eq!(request["params"]["targetId"], target, "operation targeted a different tab");
            }
            if let Some(context) =
                answer.as_object_mut().and_then(|object| object.remove("_expected_context"))
            {
                assert_eq!(
                    request["params"]["browserContextId"], context,
                    "cookies were read from the handler's old context instead of its current tab"
                );
            }
            if request["method"] == "Target.getTargets" {
                assert_eq!(
                    request["params"]["filter"],
                    json!([{}]),
                    "cleanup inherited a potentially incomplete discovery filter"
                );
            }
            if let Some(delay) = answer
                .as_object_mut()
                .and_then(|object| object.remove("_delay_ms"))
                .and_then(|value| value.as_u64())
            {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            answer["id"] = request["id"].clone();
            if socket
                .send(tokio_tungstenite::tungstenite::Message::Text(answer.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let cdp = Rc::new(
        crate::chrome::cdp::Cdp::connect(
            &format!("ws://{address}"),
            std::time::Duration::from_secs(1),
            crate::settings::Dialogs::Accept,
        )
        .await
        .unwrap(),
    );
    let host = Host::new(settings(temp.path(), temp.path()));
    *host.shared.lock().await = Some(Rc::new(Shared { cdp, identity: None, process: None }));
    (host, Script { worker, remaining })
}

struct Script {
    worker: tokio::task::JoinHandle<()>,
    remaining: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl Script {
    fn assert_done(&self) {
        assert!(!self.worker.is_finished(), "fixture transport ended unexpectedly");
        assert_eq!(
            self.remaining.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "cleanup skipped a required browser check"
        );
    }
}
impl Drop for Script {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

fn signin_cookie() -> Value {
    json!({"name":"signin","value":"private-cookie-value","domain":"www.example.test","path":"/", "secure":true,"httpOnly":true,"session":true,"expires":-1})
}

#[tokio::test]
async fn handles_survive_reordering_and_never_rebind_after_target_closure() {
    let temp = tempfile::tempdir().unwrap();
    let a = json!({"type":"page","targetId":"a","url":"https://a.test/","title":"A"});
    let b = json!({"type":"page","targetId":"b","url":"https://b.test/","title":"B"});
    let c = json!({"type":"page","targetId":"c","url":"https://c.test/","title":"C"});
    let (host, script) = scripted_host(
        &temp,
        vec![
            ("Target.getTargets", json!({"result":{"targetInfos":[a,b,c]}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[c,b,a]}})),
            ("Target.closeTarget", json!({"result":{"success":true},"_expected_target":"a"})),
            ("Target.getTargets", json!({"result":{"targetInfos":[b]}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[b]}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[b]}})),
        ],
    )
    .await;
    let mut handler = host.session();
    handler.book.registry.adopt("a");
    handler.book.registry.adopt("b");
    handler.book.ledger.borrow_mut().hold("c", crate::tabs::ledger::Hold::Owned("other".into()));
    let listed = crate::tools::tabs::tab(&mut handler, json!({"action":"list"})).await.unwrap();
    assert!(listed.contains(" t1 unchecked https://a.test/ — A") && listed.contains(" t2 "), "{listed}");
    assert!(listed.contains("1 other tab open, of session other") && !listed.contains("c.test"), "{listed}");
    crate::tools::tabs::tab(&mut handler, json!({"action":"close","id":"t1"})).await.unwrap();
    let error = crate::tools::tabs::tab(&mut handler, json!({"action":"close","id":"t1"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no tab t1"), "{error}");
    let listed = crate::tools::tabs::tab(&mut handler, json!({"action":"list"})).await.unwrap();
    assert!(listed.contains(" t2 ") && !listed.contains("t1"), "{listed}");
    script.assert_done();
}

#[tokio::test]
async fn context_cleanup_uses_its_owner_even_when_host_connection_is_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let (host, script) = scripted_host(
        &temp,
        vec![
            ("Target.disposeBrowserContext", json!({"result":{}})),
            ("Target.getBrowserContexts", json!({"result":{"browserContextIds":[]}})),
        ],
    )
    .await;
    let owner = host.shared.lock().await.take().unwrap();
    let mut handler = host.session();
    handler.book.context = Some(crate::tabs::context::BrowserContext::new(owner.cdp.clone(), "owned".into()));
    handler.book.dispose_contexts(true, |_| true).await.unwrap();
    assert!(handler.book.context.is_none());
    assert!(host.shared.lock().await.is_none());
    assert!(!host.settings().profile.exists());
    script.assert_done();
}

#[tokio::test]
async fn cookie_read_failure_cannot_replace_the_active_context() {
    for mut reply in
        [json!({"error":{"code":-32000,"message":"injected read failure"}}), json!({"result":{}})]
    {
        reply["_expected_context"] = json!("source");
        let temp = tempfile::tempdir().unwrap();
        let (host, script) = scripted_host(&temp, vec![
            ("Target.getTargetInfo", json!({"result":{"targetInfo":{"targetId":"old-tab","url":"https://www.example.test/","browserContextId":"source"}}})),
            ("Storage.getCookies", reply),
        ]).await;
        let mut handler = host.session();
        let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
        handler.book.active = Some(Tab::fixture(cdp.clone(), "old-tab".into(), "old-session".into()));
        handler.book.context = Some(crate::tabs::context::BrowserContext::new(cdp.clone(), "old".into()));
        assert!(crate::tools::context::retry_clean(&mut handler, json!({"keep_signin":true})).await.is_err());
        assert_eq!(handler.book.context.as_deref(), Some("old"));
        assert_eq!(handler.book.active.as_ref().unwrap().target, "old-tab");
        script.assert_done();
    }
}

#[tokio::test]
async fn cookie_write_failure_preserves_old_context_and_reclaims_the_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let (host, script) = scripted_host(
        &temp,
        vec![
            (
                "Target.getTargetInfo",
                json!({"result":{"targetInfo":{"targetId":"old-tab","url":"https://www.example.test/"}}}),
            ),
            ("Storage.getCookies", json!({"result":{"cookies":[signin_cookie()]}})),
            ("Target.createBrowserContext", json!({"result":{"browserContextId":"candidate"}})),
            (
                "Storage.setCookies",
                json!({"error":{"code":-32000,"message":"injected cookie write failure"}}),
            ),
            ("Target.disposeBrowserContext", json!({"result":{}})),
            ("Target.getBrowserContexts", json!({"result":{"browserContextIds":["old"]}})),
        ],
    )
    .await;
    let mut handler = host.session();
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    handler.book.active = Some(Tab::fixture(cdp.clone(), "old-tab".into(), "old-session".into()));
    handler.book.context = Some(crate::tabs::context::BrowserContext::new(cdp.clone(), "old".into()));
    let error = crate::tools::context::retry_clean(&mut handler, json!({"keep_signin":true}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("injected cookie write failure") && !error.contains("private-cookie-value"));
    assert_eq!(handler.book.context.as_deref(), Some("old"));
    assert_eq!(handler.book.active.as_ref().unwrap().target, "old-tab");
    cdp.call(None, "Target.getBrowserContexts", json!({})).await.unwrap();
    script.assert_done();
}

#[tokio::test]
async fn missing_cookies_after_write_cannot_be_reported_as_preserved_signin() {
    let temp = tempfile::tempdir().unwrap();
    let (host, script) = scripted_host(
        &temp,
        vec![
            ("Storage.setCookies", json!({"result":{}})),
            ("Storage.getCookies", json!({"result":{"cookies":[]}})),
        ],
    )
    .await;
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    let error =
        crate::tabs::cookies::restore(&cdp, "candidate", &[signin_cookie()]).await.unwrap_err().to_string();
    assert!(error.contains("verification failed") && !error.contains("private-cookie-value"));
    script.assert_done();
}

#[tokio::test]
async fn close_and_confirmation_share_one_deadline_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    // Each response fits the one-second CDP timeout individually, but their
    // combined latency exceeds the cleanup budget.
    let (host, script) = scripted_host(
        &temp,
        vec![
            ("Target.closeTarget", json!({"result":{"success":true},"_delay_ms":600})),
            ("Target.getTargets", json!({"result":{"targetInfos":[]},"_delay_ms":600})),
        ],
    )
    .await;
    let mut handler = host.session();
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    handler.book.active = Some(Tab::fixture(cdp.clone(), "target".into(), "session".into()));
    handler.book.registry.adopt("target");
    let error = crate::tools::tabs::close(&mut handler, json!({})).await.unwrap_err().to_string();
    assert!(error.contains("timed out") && error.contains("unconfirmed"), "{error}");
    assert!(handler.book.active.is_some());
    script.assert_done();
}

#[tokio::test]
async fn already_absent_resources_are_reconciled_without_replaying_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let (host, script) = scripted_host(
        &temp,
        vec![
            ("Target.closeTarget", json!({"error":{"code":-32000,"message":"arbitrary error text"}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[]}})),
            ("Target.disposeBrowserContext", json!({"error":{"code":-32000,"message":"different error"}})),
            ("Target.getBrowserContexts", json!({"result":{"browserContextIds":[]}})),
        ],
    )
    .await;
    let mut handler = host.session();
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    handler.book.active = Some(Tab::fixture(cdp.clone(), "target".into(), "session".into()));
    handler.book.registry.adopt("target");
    handler.book.context = Some(crate::tabs::context::BrowserContext::new(cdp.clone(), "context".into()));
    assert!(crate::tools::tabs::close(&mut handler, json!({})).await.unwrap().contains("Closed t1."));
    assert!(handler.book.active.is_none() && handler.book.context.is_none());
    script.assert_done();
}

#[tokio::test]
async fn incomplete_resource_lists_cannot_prove_absence() {
    for listed in [
        json!({}),
        json!({"targetInfos":null}),
        json!({"targetInfos":[{}]}),
        json!({"targetInfos":[{"targetId":""}]}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (host, script) = scripted_host(
            &temp,
            vec![
                ("Target.closeTarget", json!({"error":{"code":-32000,"message":"not found"}})),
                ("Target.getTargets", json!({"result":listed})),
            ],
        )
        .await;
        let mut handler = host.session();
        let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
        handler.book.active = Some(Tab::fixture(cdp.clone(), "target".into(), "session".into()));
        handler.book.registry.adopt("target");
        handler.book.registry.adopt("target");
        assert!(crate::tools::tabs::close(&mut handler, json!({})).await.is_err());
        assert_eq!(handler.book.active.as_ref().unwrap().target, "target");
        script.assert_done();
    }
}

#[tokio::test]
async fn failed_or_rejected_close_keeps_the_tab_until_chrome_confirms() {
    let temp = tempfile::tempdir().unwrap();
    let (host, worker) = scripted_host(
        &temp,
        vec![
            ("Target.closeTarget", json!({"error":{"code":-32000,"message":"injected close refusal"}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[{"targetId":"target"}]}})),
            ("Target.closeTarget", json!({"result":{"success":false}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[{"targetId":"target"}]}})),
            ("Target.closeTarget", json!({"result":{"success":true}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[]}})),
        ],
    )
    .await;
    let mut handler = host.session();
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    handler.book.active = Some(Tab::fixture(cdp.clone(), "target".into(), "session".into()));
    handler.book.registry.adopt("target");
    for _ in 0..2 {
        assert!(
            crate::tools::tabs::close(&mut handler, json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("not confirmed")
        );
        assert_eq!(handler.book.active.as_ref().unwrap().target, "target");
    }
    assert!(crate::tools::tabs::close(&mut handler, json!({})).await.unwrap().contains("Closed t1."));
    assert!(handler.book.active.is_none());
    worker.assert_done();
}

#[tokio::test]
async fn context_failure_after_tab_close_retains_context_and_retry_disposes_without_a_tab() {
    let temp = tempfile::tempdir().unwrap();
    let (host, worker) = scripted_host(
        &temp,
        vec![
            ("Target.closeTarget", json!({"result":{"success":true}})),
            ("Target.getTargets", json!({"result":{"targetInfos":[]}})),
            (
                "Target.disposeBrowserContext",
                json!({"error":{"code":-32000,"message":"injected disposal refusal"}}),
            ),
            ("Target.getBrowserContexts", json!({"result":{"browserContextIds":["context"]}})),
            ("Target.disposeBrowserContext", json!({"result":{}})),
            ("Target.getBrowserContexts", json!({"result":{"browserContextIds":[]}})),
        ],
    )
    .await;
    let mut handler = host.session();
    let cdp = host.shared.lock().await.as_ref().unwrap().cdp.clone();
    handler.book.active = Some(Tab::fixture(cdp.clone(), "target".into(), "session".into()));
    handler.book.registry.adopt("target");
    handler.book.context = Some(crate::tabs::context::BrowserContext::new(cdp.clone(), "context".into()));
    let error = crate::tools::tabs::close(&mut handler, json!({})).await.unwrap_err().to_string();
    assert!(error.contains("Tabs closed, but") && error.contains("injected disposal refusal"), "{error}");
    assert!(handler.book.active.is_none());
    assert_eq!(handler.book.context.as_deref(), Some("context"));
    assert!(
        crate::tools::tabs::close(&mut handler, json!({}))
            .await
            .unwrap()
            .contains("Closed this session's browser context")
    );
    assert!(handler.book.context.is_none());
    worker.assert_done();
}
