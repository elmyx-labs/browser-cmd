//! Wait and verify: several constraints as one checkpoint, a budget that
//! bounds the poll, and absence that is never proved from what could not be
//! read.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_verifies_several_constraints_at_once() {
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title><h1>Order 4182</h1>
         <p>Total: £42.00</p><p>Ships: 2026-09-09</p>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        // All three present, and one that must be absent: one pass.
        let all = session
            .execute("verify", json!({ "text": ["Order 4182", "£42.00", "2026-09-09"], "gone": "Pending" }))
            .await;
        assert!(all.ok, "{}", all.output);
        // One of several missing: the checkpoint fails naming just it.
        let miss = session.execute("verify", json!({ "text": ["Order 4182", "£43.00"] })).await;
        assert!(!miss.ok);
        assert!(miss.output.contains("\"£43.00\" is not on the page"), "{}", miss.output);
        assert!(
            !miss.output.contains("Order 4182\" is not"),
            "it blamed a phrase that held: {}",
            miss.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn wait_budget_bounds_a_poll_interval_longer_than_the_wait() {
    let address = serve("<!doctype html><button>Present</button>".into());
    on_page_with(
        address,
        |settings| {
            settings.waits.poll = std::time::Duration::from_secs(5);
        },
        async |session, _| {
            let started = std::time::Instant::now();
            let result = session.execute("wait", json!({"text":"Absent","seconds":0.05})).await;
            assert!(!result.ok, "{}", result.output);
            assert!(started.elapsed() < std::time::Duration::from_secs(1), "poll exceeded wait budget");
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_ref_whose_element_was_removed_does_not_verify() {
    let address = serve("<!doctype html><button>Delete me</button>".into());
    on_page(address, async |session, _| {
        let reference = session.document.refs.handles()[0].0.clone();
        let _ =
            session.execute("eval", json!({"expression":"document.querySelector('button').remove()"})).await;
        let result = session.execute("verify", json!({"ref":reference})).await;
        assert!(!result.ok, "removed ref verified: {}", result.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_document_without_html_body_cannot_prove_text_absence() {
    let address = serve_type("image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg' width='400' height='200'><title>Vector document</title><text x='20' y='60'>Vector evidence</text></svg>".into());
    on_page(address, async |session, _| {
        let verified = session.execute("verify", json!({"gone":"Vector evidence"})).await;
        assert!(!verified.ok && verified.output.contains("unreadable"), "{}", verified.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn failed_body_reads_never_verify_absence_and_url_checks_do_not_need_body_text() {
    let address = serve("<!doctype html><button>Main evidence</button>".into());
    on_page(address, async |session, _| {
        let tab = session.current().await.unwrap();
        // Deliberate fault injection into helpers; public eval cannot do this.
        session.document.world.evaluate(&tab, "Object.defineProperty(document, 'body', {configurable: true, get() { throw Error('fixture read unavailable') }}); true").await.unwrap();
        for (tool, args) in [("text", json!({})), ("verify", json!({"gone":"secret"})), ("wait", json!({"gone":"secret", "seconds":1}))] {
            let result = session.execute(tool, args).await;
            assert!(!result.ok && result.output.contains("fixture read unavailable"), "{tool}: {}", result.output);
        }
        let url_only = session.execute("verify", json!({"url":address.to_string()})).await;
        assert!(url_only.ok, "{}", url_only.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn an_unreadable_frame_cannot_be_treated_as_empty() {
    let address = serve(
        "<!doctype html><button>Main evidence</button><iframe srcdoc='<p>Secret remains</p>'></iframe>"
            .into(),
    );
    on_page(address, async |session, _| {
        let tab = session.current().await.unwrap();
        let tree = tab.call("Page.getFrameTree", json!({})).await.unwrap();
        let frame = &tree["frameTree"]["childFrames"][0]["frame"]["id"];
        let world = tab.call("Page.createIsolatedWorld", json!({"frameId":frame,"worldName":"util"})).await.unwrap();
        tab.call("Runtime.evaluate", json!({"contextId":world["executionContextId"],"expression":"Object.defineProperty(document, 'body', {get() { throw Error('fixture child unreadable') }}); true","returnByValue":true})).await.unwrap();
        let positive = session.execute("verify", json!({"text":"Main evidence"})).await;
        assert!(positive.ok, "{}", positive.output);
        let negative = session.execute("verify", json!({"gone":"Secret remains"})).await;
        assert!(!negative.ok && negative.output.contains("unreadable"), "{}", negative.output);
        let searched = session.execute("text", json!({"find":"Secret remains"})).await;
        assert!(searched.ok && searched.output.contains("Coverage: partial"), "{}", searched.output);
        let waited = session.execute("wait", json!({"gone":"Secret remains", "seconds":0.2})).await;
        assert!(!waited.ok && waited.output.contains("could not decide"), "{}", waited.output);
    })
    .await;
}
