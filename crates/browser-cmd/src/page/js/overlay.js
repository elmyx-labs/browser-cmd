function () {
  // Enough of a label to recognise the thing, and enough of its buttons to
  // choose one; an overlay's whole text is the page it is standing in front of.
  const LABEL_CHARS = 120;
  const CHOICES_SHOWN = 6;
  // A layer standing in front of the page rather than beside it: it is stacked
  // above (a z-index it had to ask for) and it covers a real share of the view.
  // A backdrop is the larger claim, so it asks for more of both.
  const BACKDROP_LAYER = 10;
  const BACKDROP_SHARE = 0.5;
  const OVERLAY_LAYER = 1;
  const OVERLAY_SHARE = 0.25;
  const tidy = (text) => (text || '').trim().replace(/\s+/g, ' ').slice(0, LABEL_CHARS);
  const label = (node) => {
    const by = node.getAttribute('aria-labelledby');
    const titled = by && document.getElementById(by);
    return tidy(
      node.getAttribute('aria-label')
        || (titled && titled.innerText)
        || (node.querySelector('h1, h2, h3, [role=heading]') || {}).innerText
        || node.innerText,
    );
  };
  const choices = (node) =>
    Array.from(node.querySelectorAll('button, a[href], [role=button], input[type=submit]'))
      .map((it) => tidy(it.innerText || it.value || it.getAttribute('aria-label')))
      .filter(Boolean)
      .slice(0, CHOICES_SHOWN);
  const area = innerWidth * innerHeight;
  const hit = document.elementFromPoint(Math.round(innerWidth / 2), Math.round(innerHeight / 2));
  const declared = document.querySelector('dialog[open], [aria-modal="true"], [role="alertdialog"]');
  // A page is entitled to mark a docked widget — a colour picker, a settings
  // panel — as a dialog; that alone does not block anything. It blocks only
  // when it actually intercepts the middle of the page: the centre is over it,
  // or over a large overlay standing in front of the content (a backdrop).
  if (declared && hit) {
    const s = getComputedStyle(hit);
    const box = hit.getBoundingClientRect();
    const backdrop = (s.position === 'fixed' || s.position === 'absolute')
      && (parseInt(s.zIndex) || 0) >= BACKDROP_LAYER
      && box.width * box.height >= area * BACKDROP_SHARE;
    if (declared.contains(hit) || declared === hit || backdrop) {
      return { modal: true, what: label(declared), choices: choices(declared) };
    }
  }
  for (let node = hit; node && node !== document.body; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.position !== 'fixed' && style.position !== 'sticky') continue;
    // A real overlay stacks above the page — it sets a z-index to do so. Sticky
    // content in the flow (a sticky column beside a form, a product gallery, a
    // header) leaves z-index 'auto' and blocks nothing, however large it is.
    if ((parseInt(style.zIndex) || 0) < OVERLAY_LAYER) continue;
    const box = node.getBoundingClientRect();
    if (box.width * box.height < area * OVERLAY_SHARE) continue;
    const picks = choices(node);
    if (picks.length) return { modal: false, what: label(node), choices: picks };
  }
  return null;
}
