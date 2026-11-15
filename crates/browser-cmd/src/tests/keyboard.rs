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
        assert!(
            heard.output.contains("/Slashfalse;ShiftShiftLefttrue;?Slashtrue;.Periodfalse;aKeyAfalse;"),
            "{}",
            heard.output
        );
    })
    .await;
}

/// A field is typed into key by key. Only characters outside the US layout
/// use the input-method insertion fallback.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_keyboard_events_include_modifiers_and_all_us_symbols() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <input id=i aria-label=Entry><script>
        document.documentElement.dataset.keys='[]';
        for(const type of ['keydown','keyup']) addEventListener(type,e=>{
          const keys=JSON.parse(document.documentElement.dataset.keys);
          keys.push([e.type,e.key,e.code,e.shiftKey,e.ctrlKey,e.metaKey,e.isTrusted]);
          document.documentElement.dataset.keys=JSON.stringify(keys);
        });
        i.focus();</script>"#
            .into(),
    );
    on_page(address, async |session, _| {
        let tab = session.current().await.unwrap();
        let text: String = (' '..='~').collect();
        let typed = session.execute("type", json!({"ref":"Entry","text":text})).await;
        assert!(typed.ok, "{}", typed.output);
        assert_eq!(session.document.world.evaluate(&tab, "i.value").await.unwrap(), text);
        assert_eq!(
            session
                .document
                .world
                .evaluate(&tab, "JSON.parse(document.documentElement.dataset.keys).filter(e=>e[0]==='keydown' && e[1]!=='Shift').length")
                .await
                .unwrap(),
            95
        );
        assert_eq!(session.document.world.evaluate(&tab, "JSON.parse(document.documentElement.dataset.keys).every(e=>e[6])").await.unwrap(), true);
        session.document.world.evaluate(&tab, "i.value=''; document.documentElement.dataset.keys='[]'; true").await.unwrap();
        let typed = session.execute("type", json!({"ref":"Entry","text":"Aé"})).await;
        assert!(typed.ok, "{}", typed.output);
        assert_eq!(
            session.document.world.evaluate(&tab, "JSON.parse(document.documentElement.dataset.keys).map(e=>e.slice(0,4))").await.unwrap(),
            json!([
                ["keydown", "Shift", "ShiftLeft", true],
                ["keydown", "A", "KeyA", true],
                ["keyup", "A", "KeyA", true],
                ["keyup", "Shift", "ShiftLeft", false]
            ])
        );
        assert_eq!(session.document.world.evaluate(&tab, "i.value.endsWith('Aé')").await.unwrap(), true);
        session.document.world.evaluate(&tab, "document.documentElement.dataset.keys='[]'; true").await.unwrap();
        let modifier = if cfg!(target_os = "macos") { "Meta" } else { "Control" };
        let pressed = session.execute("press", json!({"key":format!("{modifier}+a")})).await;
        assert!(pressed.ok, "{}", pressed.output);
        let events = session.document.world.evaluate(&tab, "JSON.parse(document.documentElement.dataset.keys).map(e=>e.slice(0,3))").await.unwrap();
        assert_eq!(
            events,
            json!([
                ["keydown", modifier, format!("{modifier}Left")],
                ["keydown", "a", "KeyA"],
                ["keyup", "a", "KeyA"],
                ["keyup", modifier, format!("{modifier}Left")]
            ])
        );
        assert_eq!(session.document.world.evaluate(&tab, "i.selectionStart === 0 && i.selectionEnd === i.value.length").await.unwrap(), true);
        assert_eq!(session.document.world.evaluate(&tab, "JSON.parse(document.documentElement.dataset.keys).at(-1).slice(3,6)").await.unwrap(), json!([false,false,false]));
        let pressed = session.execute("press", json!({"key":"Shift+a"})).await;
        assert!(pressed.ok, "{}", pressed.output);
        assert_eq!(session.document.world.evaluate(&tab, "i.value").await.unwrap(), "A");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_selects_options_and_rejects_disabled_choices() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <select id=s aria-label=Choice>
          <option value=a>Alpha</option><option disabled>Disabled</option>
          <optgroup label=Unavailable disabled><option>Skipped</option></optgroup>
          <option value=b>Beta</option><option value=c>Gamma</option>
        </select>
        <select id=m aria-label=Multiple multiple><option selected>One</option><option>Two</option></select>
        <select aria-label=Locked disabled><option>Locked option</option></select>
        <script>document.documentElement.dataset.events='[]';
        for(const type of ['input','change']) s.addEventListener(type,e=>{
          const events=JSON.parse(document.documentElement.dataset.events);
          events.push([e.type,e.isTrusted,s.value]);
          document.documentElement.dataset.events=JSON.stringify(events);
        });</script>"#
            .into(),
    );
    on_page(address, async |session, _| {
        let tab = session.current().await.unwrap();
        for (option, value) in [("Beta", "b"), ("Gamma", "c"), ("Alpha", "a")] {
            let selected = session.execute("select", json!({"ref":"Choice", "option":option})).await;
            assert!(selected.ok, "{}; state: {:?}", selected.output, session.document.world.evaluate(&tab, "[s.selectedIndex,s.value,document.activeElement.id,document.documentElement.dataset.events]").await);
            assert_eq!(session.document.world.evaluate(&tab, "s.value").await.unwrap(), value);
        }
        assert_eq!(
            session
                .document
                .world
                .evaluate(&tab, "JSON.parse(document.documentElement.dataset.events).every(e=>!e[1])")
                .await
                .unwrap(),
            true
        );
        assert_eq!(
            session
                .document
                .world
                .evaluate(
                    &tab,
                    "JSON.parse(document.documentElement.dataset.events).some(e=>e[0]==='change')"
                )
                .await
                .unwrap(),
            true
        );
        let disabled = session.execute("select", json!({"ref":"Choice", "option":"Disabled"})).await;
        assert!(!disabled.ok && disabled.output.contains("option is disabled"), "{}", disabled.output);
        let group = session.execute("select", json!({"ref":"Choice", "option":"Skipped"})).await;
        assert!(!group.ok && group.output.contains("option is disabled"), "{}", group.output);
        let locked = session.execute("select", json!({"ref":"Locked", "option":"Locked option"})).await;
        assert!(!locked.ok && locked.output.contains("select is disabled"), "{}", locked.output);
        let multiple = session.execute("select", json!({"ref":"Multiple", "option":"Two"})).await;
        assert!(multiple.ok, "{}", multiple.output);
        assert_eq!(
            session.document.world.evaluate(&tab, "Array.from(m.selectedOptions,o=>o.value)").await.unwrap(),
            json!(["Two"])
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_treats_crlf_as_one_enter_when_typing() {
    let address = serve(
        "<!doctype html><title>t</title><div id=e contenteditable></div><script>e.focus()</script>".into(),
    );
    on_page(address, async |session, _| {
        let typed = session.execute("type", json!({"text":"first\r\nsecond"})).await;
        assert!(typed.ok, "{}", typed.output);
        let tab = session.current().await.unwrap();
        assert_eq!(session.document.world.evaluate(&tab, "e.innerText").await.unwrap(), "first\nsecond");
    })
    .await;
}
