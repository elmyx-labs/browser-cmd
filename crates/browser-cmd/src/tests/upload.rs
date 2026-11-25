//! Handing a page a file: through the input itself, and through the button a
//! page puts in front of an input it hides or makes on the spot.

use super::*;

async fn uploads(body: &str, reference: &str) -> Outcome {
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>t</title>{body}<p id=out>none</p>
         <script>const took = (input) => out.textContent = 'got ' + input.files[0].name;</script>"
    );
    on_page(serve(page), async |session, _| {
        std::fs::write(session.settings().workspace.join("a.txt"), "hello").unwrap();
        session.execute("upload", json!({ "ref": reference, "path": "a.txt" })).await
    })
    .await
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_uploads_through_the_button_in_front_of_a_hidden_input() {
    let sent = uploads(
        r#"<input type=file id=f style="display:none" onchange="took(this)">
           <button onclick="f.click()">Choose file</button>"#,
        "Choose file",
    )
    .await;
    assert!(sent.ok && sent.output.contains("got a.txt"), "{}", sent.output);
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_uploads_through_a_button_that_makes_its_input_on_the_spot() {
    let sent = uploads(
        r#"<button onclick="const i = document.createElement('input'); i.type = 'file';
                            i.onchange = () => took(i); i.click()">Open</button>"#,
        "Open",
    )
    .await;
    assert!(sent.ok && sent.output.contains("got a.txt"), "{}", sent.output);
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_uploads_into_the_input_itself() {
    let sent =
        uploads(r#"<input type=file aria-label=Attachment onchange="took(this)">"#, "Attachment").await;
    assert!(sent.ok && sent.output.contains("got a.txt"), "{}", sent.output);
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_says_when_the_control_asked_for_no_file() {
    let sent = uploads("<button>Save</button>", "Save").await;
    assert!(!sent.ok && sent.output.contains("clicking it asked for no file"), "{}", sent.output);
}
