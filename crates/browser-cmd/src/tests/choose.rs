//! Choosing a suggestion: waited for however late it comes, picked only when
//! it is the one meant, and reported as chosen only when the field took it.

use super::*;

/// What an expression evaluates to on the page, as eval prints it.
async fn value(session: &mut Session, expression: &str) -> String {
    let read = session.execute("eval", json!({ "expression": expression })).await;
    assert!(read.ok, "{}", read.output);
    read.output
}

/// An ARIA 1.2 combobox: two places share a name, so the command asks which,
/// naming each by its ref, and the one named is chosen.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_asks_between_close_suggestions_and_chooses_the_one_named() {
    let address = serve(
        "<!doctype html><title>t</title><label for=c>City</label>
         <input id=c role=combobox aria-expanded=false aria-controls=lb aria-autocomplete=list>
         <ul id=lb role=listbox hidden></ul>
         <script>
         const all = ['Raleigh, NC, USA', 'Raleigh, MS, USA', 'Reno, NV, USA'];
         c.oninput = () => {
           lb.innerHTML = '';
           for (const t of all) if (c.value && t.toLowerCase().startsWith(c.value.toLowerCase())) {
             const li = document.createElement('li');
             li.setAttribute('role', 'option'); li.textContent = t;
             li.onclick = () => { c.value = t; lb.hidden = true; c.setAttribute('aria-expanded', 'false'); };
             lb.append(li);
           }
           lb.hidden = !lb.children.length;
           c.setAttribute('aria-expanded', String(!lb.hidden));
         };
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let typed = session.execute("type", json!({ "ref": "City", "text": "Ral" })).await;
        assert!(typed.output.contains("choose City \"TEXT\""), "{}", typed.output);
        let asked = session.execute("choose", json!({ "ref": "City", "text": "Raleigh" })).await;
        assert!(!asked.ok, "{}", asked.output);
        for want in ["\"Raleigh, NC, USA\"", "\"Raleigh, MS, USA\"", "option=", "@e"] {
            assert!(asked.output.contains(want), "{want}: {}", asked.output);
        }
        let chose = session
            .execute("choose", json!({ "ref": "City", "text": "Raleigh", "option": "Raleigh, MS, USA" }))
            .await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("Chose \"Raleigh, MS, USA\" in City"), "{}", chose.output);
        assert!(value(session, "c.value").await.contains("Raleigh, MS, USA"));
    })
    .await;
}

/// A controlled field that debounces, fetches late, shows an earlier answer
/// meanwhile, renders its options in a portal and clears itself on a blur
/// that chose nothing: the late list is the one picked, and the value stays.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_waits_out_a_debounced_fetch_past_a_stale_list() {
    let address = serve(
        "<!doctype html><title>t</title>
         <label>Where do you live? <input id=w role=combobox aria-autocomplete=list></label><button>Continue</button>
         <script>
         let timer, committed = false, menu = null;
         const show = (items) => {
           menu?.remove();
           menu = document.createElement('div');
           menu.setAttribute('role', 'listbox');
           menu.onmousedown = (e) => e.preventDefault();
           for (const t of items) {
             const o = document.createElement('div');
             o.setAttribute('role', 'option'); o.textContent = t;
             o.onclick = () => { w.value = t; committed = true; menu.remove(); menu = null; };
             menu.append(o);
           }
           document.body.append(menu);
         };
         w.oninput = () => {
           committed = false;
           show(['Boston, MA, USA']);
           clearTimeout(timer);
           const q = w.value.toLowerCase();
           timer = setTimeout(() => setTimeout(() => show(['Raleigh, NC, USA', 'Rome, Italy'].filter((p) => p.toLowerCase().startsWith(q))), 600), 400);
         };
         w.onblur = () => { if (!committed) w.value = ''; menu?.remove(); menu = null; };
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let chose =
            session.execute("choose", json!({ "ref": "Where do you live?", "text": "Raleigh" })).await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("clicked suggestion 1 of 1"), "{}", chose.output);
        let next = session.execute("click", json!({ "ref": "Continue" })).await;
        assert!(next.ok, "{}", next.output);
        assert!(value(session, "w.value").await.contains("Raleigh, NC, USA"));
    })
    .await;
}

/// Google Places draws its suggestions as role-less rows in a container at
/// the end of the body, and takes one on mousedown.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_chooses_from_a_list_without_roles() {
    let address = serve(
        "<!doctype html><title>t</title><input id=p aria-label=Location placeholder='Enter a location'>
         <div class=pac-container style='display:none'></div>
         <script>
         const box = document.querySelector('.pac-container');
         p.oninput = () => setTimeout(() => {
           box.innerHTML = '';
           for (const [main, rest] of [['Raleigh', 'NC, USA'], ['Raleigh-Durham International Airport', 'Morrisville, NC, USA']]) {
             const d = document.createElement('div');
             d.className = 'pac-item';
             d.innerHTML = `<span class=pac-icon></span><span class=pac-item-query><span class=pac-matched>${main.slice(0, 3)}</span>${main.slice(3)}</span> <span>${rest}</span>`;
             d.onmousedown = () => { p.value = `${main}, ${rest}`; box.style.display = 'none'; };
             box.append(d);
           }
           const r = p.getBoundingClientRect();
           box.style.cssText = `display:block;position:absolute;top:${r.bottom}px;left:${r.left}px`;
         }, 300);
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let chose = session.execute("choose", json!({ "ref": "Location", "text": "Raleigh, NC" })).await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("Chose \"Raleigh NC, USA\""), "{}", chose.output);
        assert!(chose.output.contains("the list closed"), "{}", chose.output);
        assert!(value(session, "p.value").await.contains("Raleigh, NC, USA"));
    })
    .await;
}

/// A postcode the widget does not know: nothing to pick, and the page's own
/// words for it are quoted.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_says_when_no_suggestion_comes() {
    let address = serve(
        "<!doctype html><title>t</title><label>ZIP or city <input id=z role=combobox aria-controls=zl></label>
         <div id=zl role=listbox></div><p id=msg></p>
         <script>z.oninput = () => setTimeout(() => { msg.textContent = 'No results found for ' + z.value; }, 200);</script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let none = session.execute("choose", json!({ "ref": "ZIP or city", "text": "27615" })).await;
        assert!(!none.ok, "{}", none.output);
        assert!(none.output.contains("no suggestions appeared"), "{}", none.output);
        assert!(none.output.contains("No results found for 27615"), "{}", none.output);
    })
    .await;
}

/// A multi-select keeps its choices as chips beside an empty field, where a
/// Backspace would take the last one away: the new one joins them.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_adds_a_chip_without_removing_the_last() {
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title><div id=tags><span class=chip>Durham ×</span>
         <input id=t aria-label=Cities role=combobox aria-controls=tl></div>
         <ul id=tl role=listbox aria-multiselectable=true></ul>
         <script>
         t.oninput = () => {
           tl.innerHTML = '';
           for (const n of ['Raleigh', 'Richmond', 'Durham']) if (t.value && n.toLowerCase().startsWith(t.value.toLowerCase())) {
             const li = document.createElement('li');
             li.setAttribute('role', 'option'); li.textContent = n;
             li.onmousedown = (e) => e.preventDefault();
             li.onclick = () => { const s = document.createElement('span'); s.className = 'chip'; s.textContent = n + ' ×'; t.before(s); t.value = ''; tl.innerHTML = ''; };
             tl.append(li);
           }
         };
         t.onkeydown = (e) => { if (e.key === 'Backspace' && !t.value) t.parentElement.querySelector('.chip:last-of-type')?.remove(); };
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let chose = session.execute("choose", json!({ "ref": "Cities", "text": "Ral" })).await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("it now shows by the field"), "{}", chose.output);
        let chips =
            value(session, "[...document.querySelectorAll('.chip')].map((c) => c.textContent).join('|')")
                .await;
        assert!(chips.contains("Durham ×|Raleigh ×"), "{chips}");
    })
    .await;
}

/// A native select is selected in, and a datalist is chosen from by typing
/// its whole value.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_chooses_in_a_native_select_and_a_datalist() {
    let address = serve(
        "<!doctype html><title>t</title>
         <label>Size <select id=s><option>Small</option><option>Medium</option></select></label>
         <label>Fruit <input id=f list=fl></label>
         <datalist id=fl><option value=Apple><option value=Apricot><option value=Banana></datalist>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let selected = session.execute("choose", json!({ "ref": "Size", "text": "Med" })).await;
        assert!(selected.ok && selected.output.contains("Selected \"Medium\""), "{}", selected.output);
        let listed = session.execute("choose", json!({ "ref": "Fruit", "text": "Apr" })).await;
        assert!(listed.ok, "{}", listed.output);
        assert!(listed.output.contains("Chose \"Apricot\" in Fruit from its list of 3"), "{}", listed.output);
        assert!(value(session, "[s.value, f.value].join('|')").await.contains("Medium|Apricot"));
    })
    .await;
}

/// A combobox inside a web component's open shadow root.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_chooses_in_a_shadow_root() {
    let address = serve(
        "<!doctype html><title>t</title><town-picker></town-picker>
         <script>
         customElements.define('town-picker', class extends HTMLElement {
           connectedCallback() {
             const r = this.attachShadow({ mode: 'open' });
             r.innerHTML = '<label for=i>Town</label><input id=i role=combobox aria-controls=l aria-expanded=false><ul id=l role=listbox></ul>';
             const i = r.getElementById('i'), l = r.getElementById('l');
             i.oninput = () => {
               l.innerHTML = '';
               for (const n of ['Raleigh', 'Reno']) if (i.value && n.toLowerCase().startsWith(i.value.toLowerCase())) {
                 const o = document.createElement('li');
                 o.setAttribute('role', 'option'); o.textContent = n;
                 o.onclick = () => { i.value = n; l.innerHTML = ''; i.setAttribute('aria-expanded', 'false'); };
                 l.append(o);
               }
               i.setAttribute('aria-expanded', String(!!l.children.length));
             };
           }
         });
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let chose = session.execute("choose", json!({ "ref": "Town", "text": "Ren" })).await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("Chose \"Reno\" in Town"), "{}", chose.output);
    })
    .await;
}

/// A widget whose options ignore a click: never reported as chosen.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_says_when_a_pick_does_not_take() {
    let address = serve(
        "<!doctype html><title>t</title><label>State <input id=s role=combobox aria-controls=sl></label>
         <ul id=sl role=listbox></ul>
         <script>s.oninput = () => { sl.innerHTML = s.value ? '<li role=option>North Carolina</li><li role=option>North Dakota</li>' : ''; };</script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let stuck = session.execute("choose", json!({ "ref": "State", "text": "North Car" })).await;
        assert!(!stuck.ok, "{}", stuck.output);
        for want in ["did not take", "reads \"North Car\"", "still open"] {
            assert!(stuck.output.contains(want), "{want}: {}", stuck.output);
        }
    })
    .await;
}

/// Options that ignore a click but follow the keyboard: walked to with
/// ArrowDown and taken with Enter once the one meant is active.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_picks_with_the_keyboard_when_a_click_does_not_take() {
    let address = serve(
        "<!doctype html><title>t</title><label>State <input id=s role=combobox aria-controls=sl></label>
         <ul id=sl role=listbox></ul>
         <script>
         let at = -1;
         s.oninput = () => { at = -1; sl.innerHTML = s.value ? '<li role=option id=o0>North Carolina</li><li role=option id=o1>North Dakota</li>' : ''; };
         s.onkeydown = (e) => {
           const o = sl.children;
           if (e.key === 'ArrowDown' && o.length) { at = (at + 1) % o.length; s.setAttribute('aria-activedescendant', o[at].id); }
           if (e.key === 'Enter' && at >= 0) { s.value = o[at].textContent; sl.innerHTML = ''; }
         };
         sl.onmousedown = (e) => e.preventDefault();
         </script>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let chose = session.execute("choose", json!({ "ref": "State", "text": "North Dak" })).await;
        assert!(chose.ok, "{}", chose.output);
        assert!(chose.output.contains("picked suggestion 2 of 2 with the keyboard"), "{}", chose.output);
        assert!(value(session, "s.value").await.contains("North Dakota"));
    })
    .await;
}
