//! Typing into a real page: fields that take a value rather than keystrokes,
//! text that replaces what a field held, rich editors and shadow inputs.

use super::*;

/// A date input takes digits segment by segment in the locale's order, so
/// typing "2026-09-07" into it is nothing at all: its value is set instead.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_fills_a_date_and_a_range() {
    let address = serve(
        "<!doctype html><title>t</title>
         <label>Check-in <input type=date id=d></label>
         <label>Guests <input type=range id=r min=1 max=9></label>
         <label>Colour <input type=color id=c></label>
         <label>At <input type=time id=t></label>
         <output id=o></output>
         <script>for (const e of [d,r,c,t]) e.addEventListener('change', () => o.textContent += e.value + ';')</script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        for (field, text) in
            [("Check-in", "2026-09-07"), ("Guests", "4"), ("Colour", "#ff8800"), ("At", "18:30")]
        {
            let typed = session.execute("type", json!({ "ref": field, "text": text })).await;
            assert!(typed.ok, "{}", typed.output);
        }
        let heard = session
            .execute("eval", json!({ "expression": "[d.value, r.value, c.value, t.value, o.textContent]" }))
            .await;
        assert!(heard.ok, "{}", heard.output);
        for want in ["\"2026-09-07\"", "\"4\"", "\"#ff8800\"", "\"18:30\"", "2026-09-07;4;#ff8800;18:30;"] {
            assert!(heard.output.contains(want), "{want} not set or not announced: {}", heard.output);
        }
        let refused = session.execute("type", json!({ "ref": "Check-in", "text": "next week" })).await;
        assert!(!refused.ok);
        assert!(refused.output.contains("it takes a date as YYYY-MM-DD"), "{}", refused.output);
    })
    .await;
}

/// The select-all chord has to reach the editor as the editing command it stands
/// for, or a headless Chrome deletes one character and appends the rest.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_typing_replaces_what_a_field_already_holds() {
    let address = serve(
        r#"<!doctype html><title>t</title><label>Promo <input id="p" value="SAVE10"></label><label>Notes <textarea id="t">old text here</textarea></label>"#
            .into(),
    );
    on_page(address, async |session, _| {
        for (field, text) in [("Promo", "SAVE10"), ("Promo", "NEW"), ("Notes", "replaced")] {
            let typed = session.execute("type", json!({ "ref": field, "text": text })).await;
            assert!(typed.ok, "{}", typed.output);
        }
        let values = session.execute("eval", json!({ "expression": "[document.getElementById('p').value, document.getElementById('t').value]" })).await;
        assert!(values.output.contains("[\"NEW\",\"replaced\"]") || values.output.contains("[\n    \"NEW\",\n    \"replaced\"\n  ]") || (values.output.contains("\"NEW\"") && values.output.contains("\"replaced\"") && !values.output.contains("SAVE")), "{}", values.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_types_into_a_contenteditable() {
    // A rich-editor's editable surface, with text already in it. Clearing it
    // with select-all + Backspace hangs the key event in headless Chrome, so
    // this both proves the replace and guards against the hang (it would time
    // the test out).
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title><div contenteditable role=textbox aria-label='editor' style='border:1px solid'>Old draft</div>".to_owned(),
    );
    on_page(address, async |session, _| {
        let typed = session.execute("type", json!({ "ref": "editor", "text": "New words here" })).await;
        assert!(typed.ok, "typing into the contenteditable failed: {}", typed.output);
        let read = session
            .execute("eval", json!({ "expression": "document.querySelector('[contenteditable]').innerText" }))
            .await;
        assert!(read.output.contains("New words here"), "the new text is not there: {}", read.output);
        assert!(!read.output.contains("Old draft"), "the old text was not cleared: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_types_into_a_shadow_input() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <my-field></my-field><p id="out">idle</p>
        <script>
          customElements.define('my-field', class extends HTMLElement {
            connectedCallback() {
              const r = this.attachShadow({ mode: 'open' });
              const i = document.createElement('input');
              i.setAttribute('aria-label', 'Email');
              i.addEventListener('input', () => { document.querySelector('#out').textContent = i.value; });
              r.appendChild(i);
            }
          });
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let typed = session.execute("type", json!({ "ref": "Email", "text": "a@b.com" })).await;
        assert!(typed.ok, "typing into a shadow-root input failed: {}", typed.output);
        let read = session
            .execute("eval", json!({ "expression": "document.querySelector('#out').textContent" }))
            .await;
        assert!(read.output.contains("a@b.com"), "the shadow input was not filled: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reports_a_field_that_does_not_read_what_was_typed() {
    let address = serve(
        "<!doctype html><title>t</title>
         <label>Code <input id=c maxlength=3></label>
         <label>Name <input id=n></label>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let short = session.execute("type", json!({ "ref": "Code", "text": "12345" })).await;
        assert!(short.ok, "{}", short.output);
        assert!(short.output.starts_with("Typed; field now reads \"123\"."), "{}", short.output);
        let whole = session.execute("type", json!({ "ref": "Name", "text": "Ada" })).await;
        assert!(
            whole.output.starts_with("Typed.") && !whole.output.contains("now reads"),
            "{}",
            whole.output
        );
    })
    .await;
}

/// Text with no ref goes into the focused editor, a line break as Enter.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_types_into_the_focused_editor_without_a_ref() {
    let address = serve(
        "<!doctype html><title>t</title><div id=ed contenteditable style='min-height:40px'></div>
         <script>ed.focus()</script>"
            .into(),
    );
    on_page(address, async |session, _| {
        let typed = session.execute("type", json!({ "text": "first line\nsecond" })).await;
        assert!(typed.ok && typed.output.starts_with("Typed; 17 characters"), "{}", typed.output);
        let tab = session.current().await.unwrap();
        let text = session.document.world.evaluate(&tab, "ed.innerText").await.unwrap();
        assert_eq!(text.as_str().map(str::trim), Some("first line\nsecond"), "{text}");
    })
    .await;
}

/// A site's shortcut is often a symbol: `/` to search, `?` for help.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_presses_a_symbol_key() {
    let address = serve(
        "<!doctype html><title>t</title><output id=o></output>
         <script>addEventListener('keydown', e => o.textContent += e.key + e.code + e.shiftKey + ';')</script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        for key in ["/", "?", ".", "A"] {
            let pressed = session.execute("press", json!({ "key": key })).await;
            assert!(pressed.ok, "{}", pressed.output);
        }
        let heard = session.execute("eval", json!({ "expression": "o.textContent" })).await;
        assert!(heard.output.contains("/Slashfalse;?Slashtrue;.Periodfalse;aKeyAfalse;"), "{}", heard.output);
    })
    .await;
}
