//! What an action answers with: what it changed, what now stands over the
//! page, and a light state that neither repeats URLs nor mistakes progress
//! for a loop.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_says_what_an_action_changed_without_a_second_read() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <p id=msg>Nothing ordered yet.</p>
        <input id=promo aria-label="Promo code">
        <button id=cancel>Cancel</button>
        <button id=order>Place order</button>
        <script>
        document.getElementById('order').onclick = () => {
            document.getElementById('msg').textContent = 'Order placed: thank you';
            document.getElementById('cancel').remove();
            document.getElementById('promo').value = 'SAVE10';
        };
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        // A page just arrived at says everything for the first time, and
        // reporting all of it as news would say nothing.
        assert!(!opened.output.contains("The page now says"), "{}", opened.output);
        assert!(!opened.output.contains("Gone:"), "{}", opened.output);

        let clicked = session.execute("click", json!({ "ref": "Place order" })).await;
        assert!(clicked.ok, "{}", clicked.output);
        assert!(
            clicked.output.contains("The page now says:\n  Order placed: thank you"),
            "the message the click produced is not in the reply: {}",
            clicked.output
        );
        assert!(
            !clicked.output.contains("Nothing ordered yet"),
            "what the page already said is not news: {}",
            clicked.output
        );
        assert!(
            clicked.output.contains("Gone: button \"Cancel\""),
            "the control that went away is not named: {}",
            clicked.output
        );
        assert!(
            clicked.output.contains("[changed]") && clicked.output.contains("[value=\"SAVE10\"]"),
            "the field that now reads differently is not marked: {}",
            clicked.output
        );
        // The page's own words stay inside the fence that makes them quotable.
        let (_, fenced) = clicked.output.split_once("<browser").expect("a fenced state");
        assert!(fenced.contains("Order placed: thank you"), "{}", clicked.output);
    })
    .await;
}

/// A new document and an explicit snapshot print the whole tree.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_action_on_the_same_document_prints_only_what_changed() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <h1>Basket</h1><button>Keep one</button><button>Keep two</button>
        <button id=add onclick="document.body.insertAdjacentHTML('beforeend','<h2>Added</h2><button>Remove</button>')">Add</button>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(
            opened.output.contains("Keep one") && !opened.output.contains("not repeated"),
            "{}",
            opened.output
        );
        let clicked = session.execute("click", json!({ "ref": "Add" })).await;
        assert!(clicked.ok, "{}", clicked.output);
        assert!(clicked.output.contains("- heading \"Added\" [new]"), "{}", clicked.output);
        assert!(clicked.output.contains("  - button \"Remove\" @e"), "{}", clicked.output);
        assert!(
            clicked.output.contains("3 unchanged controls not repeated — `snapshot` reprints"),
            "{}",
            clicked.output
        );
        assert!(
            !clicked.output.contains("Keep one"),
            "unchanged controls are not repeated: {}",
            clicked.output
        );
        assert!(
            session.execute("click", json!({ "ref": "Keep one" })).await.ok,
            "an unrepeated ref still resolves"
        );
        let snapshot = session.execute("snapshot", json!({})).await;
        assert!(
            snapshot.output.contains("Keep one") && !snapshot.output.contains("not repeated"),
            "{}",
            snapshot.output
        );
        let reopened = session.execute("navigate", json!({ "url": format!("http://{address}/again") })).await;
        assert!(
            reopened.output.contains("Keep one") && !reopened.output.contains("not repeated"),
            "{}",
            reopened.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_action_state_omits_urls_but_snapshot_keeps_them() {
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title>\
         <a href=\"https://example.com/deep/link\">Open the docs</a>"
            .to_owned(),
    );
    on_page(address, async |session, nav| {
        assert!(nav.output.contains("Open the docs") && nav.output.contains(" @e"), "no ref: {}", nav.output);
        assert!(
            !nav.output.contains("example.com/deep/link"),
            "the action state should omit the URL: {}",
            nav.output
        );
        let snap = session.execute("snapshot", json!({})).await;
        assert!(
            snap.output.contains("example.com/deep/link"),
            "snapshot should carry the URL: {}",
            snap.output
        );
        // wait returns the same light state — refs to act on, no URLs — so
        // the agent need not snapshot after it. One link is shorter than the
        // count that would stand for it, so the tree itself is repeated.
        let waited = session.execute("wait", json!({ "seconds": 0.2 })).await;
        assert!(waited.output.contains(" @e"), "wait should carry the controls: {}", waited.output);
        assert!(
            !waited.output.contains("example.com/deep/link"),
            "wait state should omit URLs: {}",
            waited.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn changing_input_values_are_not_reported_as_stalled_or_cycling() {
    let address = serve("<!doctype html><input aria-label='Quantity'>".into());
    on_page(address, async |session, _| {
        for n in 1..=6 {
            let changed = session.execute("type", json!({"ref":"Quantity","text":n.to_string()})).await;
            assert!(changed.ok, "{}", changed.output);
            assert!(
                !changed.output.contains("cycle") && !changed.output.contains("repeated the same action"),
                "{}",
                changed.output
            );
        }
        for _ in 0..5 {
            let read =
                session.execute("eval", json!({"expression":"document.querySelector('input').value"})).await;
            assert!(read.ok && !read.output.contains("accessibility state"), "{}", read.output);
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_names_what_stands_over_the_page_without_answering_it() {
    let address = serve(
        r#"<!doctype html><title>t</title>
            <h1>Articles</h1><p id="under">the article nobody can reach yet</p>
            <div role="dialog" aria-modal="true" aria-label="We value your privacy"
                 style="position:fixed;inset:0;background:#fff">
              <h2>We value your privacy</h2>
              <button onclick="document.body.dataset.chose='all'">Accept all</button>
              <button onclick="document.body.dataset.chose='none'">Reject all</button>
              <button onclick="document.body.dataset.chose='some'">Manage settings</button>
            </div>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(
            opened.output.contains("A dialog is over the page"),
            "the interruption was not named: {}",
            opened.output
        );
        assert!(
            opened.output.contains("We value your privacy"),
            "it was not named by what it calls itself: {}",
            opened.output
        );
        assert!(
            opened.output.contains("Accept all, Reject all, Manage settings"),
            "its choices were not offered in order: {}",
            opened.output
        );

        let chose = session
            .execute("eval", json!({ "expression": "document.body.dataset.chose || 'nothing'" }))
            .await;
        assert!(chose.output.contains("nothing"), "something answered it for the person: {}", chose.output);

        // Answered deliberately, it stops being reported.
        let clicked = session.execute("click", json!({ "ref": "Reject all" })).await;
        assert!(clicked.ok, "{}", clicked.output);
        let after = session.execute("eval", json!({ "expression": "document.body.dataset.chose" })).await;
        assert!(after.output.contains("none"), "{}", after.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_flags_a_blocking_modal_but_not_a_docked_dialog() {
    let modal = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="position:fixed;inset:0;background:rgba(0,0,0,.5);z-index:100"></div>
        <div role="dialog" aria-modal="true" style="position:fixed;left:35%;top:35%;z-index:101;background:#fff;padding:20px">
          <h2>Accept cookies?</h2><button>Accept</button><button>Reject</button></div>"#
            .to_owned(),
    );
    let docked = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div role="dialog" aria-modal="true" style="position:fixed;left:0;top:0;width:180px;height:40px;z-index:100;background:#eee">
          <span>Stroke</span><button>Red</button></div>
        <div style="height:2000px">the canvas beneath, fully usable</div>"#
            .to_owned(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut session = Host::new(settings).session();
            let m = session.execute("navigate", json!({ "url": format!("http://{modal}") })).await;
            assert!(m.output.contains("is over the page"), "a real modal must be flagged: {}", m.output);
            let d = session.execute("navigate", json!({ "url": format!("http://{docked}") })).await;
            assert!(
                !d.output.contains("is over the page"),
                "a docked dialog must not be flagged: {}",
                d.output
            );
            let _ = session.execute("close", json!({})).await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_flags_a_stacking_overlay_but_not_sticky_content() {
    let overlay = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="position:fixed;inset:0;z-index:9999;background:#fff">
          <h2>Before you continue</h2><button>Accept</button><button>Reject</button></div>"#
            .to_owned(),
    );
    let sticky = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="position:sticky;top:0;width:100%;height:700px">
          <h2>Choose your size</h2><button>13-inch</button><button>15-inch</button></div>
        <div style="height:2000px">the rest of the page</div>"#
            .to_owned(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut session = Host::new(settings).session();
            let o = session.execute("navigate", json!({ "url": format!("http://{overlay}") })).await;
            assert!(
                o.output.contains("is over the page"),
                "a stacking overlay must be flagged: {}",
                o.output
            );
            let s = session.execute("navigate", json!({ "url": format!("http://{sticky}") })).await;
            assert!(
                !s.output.contains("is over the page"),
                "in-flow sticky content must not be flagged: {}",
                s.output
            );
            let _ = session.execute("close", json!({})).await;
        })
        .await;
}
