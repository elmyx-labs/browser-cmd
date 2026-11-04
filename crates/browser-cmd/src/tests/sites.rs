//! Where the agent may act: outside the sites allowed a page is read, not
//! clicked, typed into or scripted.

use super::*;

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Fixture</title>
    <button onclick="document.title='clicked'">Go</button>"#;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn outside_the_sites_allowed_a_page_is_read_but_not_acted_on() {
    let address = serve(PAGE.to_owned());
    on_page_with(
        address,
        |settings| settings.sites = vec!["example.test".into()],
        async |session, _| {
            let read = session.execute("snapshot", json!({})).await;
            assert!(read.ok && read.output.contains("Go"), "{}", read.output);
            let refused = session.execute("click", json!({ "ref": "Go" })).await;
            assert!(!refused.ok, "{}", refused.output);
            assert!(
                refused.output.contains("acting on 127.0.0.1 is outside the sites allowed (example.test)"),
                "{}",
                refused.output
            );
            let scripted = session.execute("eval", json!({ "expression": "document.title" })).await;
            assert!(
                !scripted.ok && scripted.output.contains("outside the sites allowed"),
                "{}",
                scripted.output
            );
        },
    )
    .await;
    on_page_with(
        address,
        |settings| settings.sites = vec!["127.0.0.1".into()],
        async |session, _| {
            let clicked = session.execute("click", json!({ "ref": "Go" })).await;
            assert!(clicked.ok, "{}", clicked.output);
        },
    )
    .await;
}
