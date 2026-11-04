//! What a ref names: a handle that outlived its element or its document is
//! refused rather than rebound, and a name that matches twice picks nothing.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_named_click_refreshes_but_stale_handles_and_ambiguity_refuse() {
    let addr = serve(r#"<!doctype html><title>Refresh</title>
        <button id='choose' onclick="document.getElementById('count').textContent=Number(document.getElementById('count').textContent)+1">Choose</button>
        <output id='count'>0</output>"#.into());
    on_page(addr, async |session, _| {
        let node = session.document.refs.resolve("Choose", &["button"]).unwrap();
        let old = format!("@{}", session.document.refs.handle_for(node).unwrap());
        let tab = session.current().await.unwrap();
        session.document.world.evaluate(&tab, "document.getElementById('choose').replaceWith(document.getElementById('choose').cloneNode(true))").await.unwrap();
        let stale = session.execute("click", json!({"ref":old})).await;
        assert!(!stale.ok, "a stale handle must not silently rebind");
        assert!(stale.output.contains("it is now @"), "but the replacement is named: {}", stale.output);
        let fresh = session.execute("click", json!({"ref":"Choose"})).await;
        assert!(fresh.ok, "{}", fresh.output);
        assert_eq!(session.document.world.evaluate(&tab, "document.getElementById('count').textContent").await.unwrap(), "1");
        session.document.world.evaluate(&tab, "document.body.append(document.getElementById('choose').cloneNode(true))").await.unwrap();
        let ambiguous = session.execute("click", json!({"ref":"Choose"})).await;
        assert!(!ambiguous.ok, "an ambiguous fresh name must not pick a button");
        assert_eq!(session.document.world.evaluate(&tab, "document.getElementById('count').textContent").await.unwrap(), "1");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn cached_refs_follow_frame_documents_without_invalidating_unrelated_or_same_document_refs() {
    let address = serve(r#"<!doctype html><button>Main</button><iframe id="child" srcdoc="<button>Child</button><iframe srcdoc='&lt;button&gt;Grandchild&lt;/button&gt;'></iframe>"></iframe>"#.into());
    on_page(address, async |handler, _| {
        let refs = handler.document.refs.references();
        let find = |name| refs.iter().find(|reference| reference.name == name).unwrap().clone();
        let main = find("Main");
        let child = find("Child");
        let grandchild = find("Grandchild");
        let tab = handler.current().await.unwrap();
        let before = tab.cdp.events().navigation_epoch(&tab.session);
        handler
            .document
            .world
            .evaluate(&tab, "history.pushState(null, '', '#same-document'); true")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while tab.cdp.events().navigation_epoch(&tab.session) == before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(handler.node(&main.handle).is_ok());
        assert!(handler.node(&child.handle).is_ok());
        let epoch = tab.cdp.events().frame_document(&tab.session, &child.frame_id);
        let replacement =
            "<button onclick=\"parent.document.body.dataset.clicked='yes'\">Replacement</button>";
        handler
            .document
            .world
            .evaluate(&tab, &format!("document.getElementById('child').srcdoc={}; true", json!(replacement)))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while tab.cdp.events().frame_document(&tab.session, &child.frame_id) == epoch {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(handler.node(&child.handle).is_err(), "cached child ref survived a document replacement");
        assert!(
            handler.node(&grandchild.handle).is_err(),
            "descendant ref survived replacement of its ancestor"
        );
        assert!(handler.node(&main.handle).is_ok(), "unrelated main-document ref was invalidated");
        let rejected = handler.execute("click", json!({"ref":format!("@{}", child.handle)})).await;
        assert!(!rejected.ok && rejected.output.contains("document binding"), "{}", rejected.output);
        assert_eq!(
            handler
                .document
                .world
                .evaluate(&tab, "document.body.dataset.clicked || 'untouched'")
                .await
                .unwrap(),
            "untouched"
        );
        let accepted = handler.execute("click", json!({"ref":format!("@{}", main.handle)})).await;
        assert!(accepted.ok, "{}", accepted.output);
        tab.call("Page.navigate", json!({"url":format!("http://{address}/next")})).await.unwrap();
        handler.settle(&tab).await;
        assert!(
            handler.node(&main.handle).is_err(),
            "cached main ref survived external navigation without a snapshot"
        );
    })
    .await;
}
