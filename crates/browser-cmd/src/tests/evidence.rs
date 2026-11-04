//! Evidence of what a page shows: one element pictured, which strings are in
//! view and where, and a raw document said for what it is.

use super::*;

/// Visible, out of view, hidden and absent strings are each told apart.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reports_which_strings_are_in_view_and_where() {
    let address = serve(
        "<!doctype html><title>t</title><h1>Weather in <b>Belgrade</b></h1>
         <p style='display:none'>Secret</p><input value='typed words'>
         <p style='margin-top:4000px'>Paris</p>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let values = json!(["weather in belgrade", "Paris", "Secret", "typed", "Nowhere"]);
        let found = session.execute("evidence", json!({ "values": values })).await;
        assert!(found.ok, "{}", found.output);
        let line = |value: &str| {
            found
                .output
                .lines()
                .find(|line| line.starts_with(&format!("- \"{value}\"")))
                .unwrap_or_default()
                .to_owned()
        };
        assert!(
            line("weather in belgrade").contains("visible at")
                && line("weather in belgrade").contains("<h1>"),
            "{}",
            found.output
        );
        assert!(line("Paris").contains("below the viewport"), "{}", found.output);
        assert!(line("Secret").contains("not rendered"), "{}", found.output);
        assert!(line("typed").contains("1 form field"), "{}", found.output);
        assert!(line("Nowhere").contains("not in the page text"), "{}", found.output);
        assert!(!found.output.contains("[warning]"), "{}", found.output);
    })
    .await;
}

/// One element is pictured at its own size, found by ref or by selector.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_pictures_one_element() {
    let address = serve(
        "<!doctype html><title>t</title><style>body{margin:0}</style>
         <div id=box style='width:200px;height:100px;margin-top:2000px;background:#08f'>Box</div>
         <button style='width:120px;height:40px'>Buy</button>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let size = |outcome: &Outcome| {
            let png = crate::text::base64::unbase64(&outcome.images[0].data).unwrap();
            let number = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
            (number(16), number(20))
        };
        let boxed = session.execute("screenshot", json!({ "selector": "#box" })).await;
        assert!(boxed.ok, "{}", boxed.output);
        assert!(boxed.output.contains("selector \"#box\", 200x100"), "{}", boxed.output);
        assert_eq!(size(&boxed), (200, 100));
        let button = session.execute("screenshot", json!({ "ref": "Buy" })).await;
        assert!(button.ok, "{}", button.output);
        assert_eq!(size(&button), (120, 40));
        let missing = session.execute("screenshot", json!({ "selector": "#nothing" })).await;
        assert!(!missing.ok && missing.output.contains("nothing on the page matches"), "{}", missing.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_warns_that_a_raw_json_page_is_poor_evidence() {
    let address = serve_type("application/json", r#"{"price": 12}"#.to_owned());
    on_page(address, async |session, _| {
        let shot = session.execute("screenshot", json!({})).await;
        assert!(
            shot.output
                .starts_with("[warning] evidence page is raw JSON; open a rendered page for a screenshot"),
            "{}",
            shot.output
        );
        let found = session.execute("evidence", json!({ "values": ["12"] })).await;
        assert!(found.output.starts_with("[warning] evidence page is raw JSON"), "{}", found.output);
    })
    .await;
}
