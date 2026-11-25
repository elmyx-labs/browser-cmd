function(x, y) {
    // Enough of the cover to recognise and act on it; its whole
    // text is the dialog, which the state below already carries.
    const COVER_CHARS = 160;
    const CONTROLS = 'a[href],button,input,select,textarea,[tabindex],[role=button],[role=link],[role=checkbox],[role=radio],[role=option],[role=combobox],[role=textbox]';
    // Inside a frame the point is in the top page's coordinates,
    // which this frame's elementFromPoint cannot read; trust the
    // box model that placed it and do not second-guess the cover.
    if (this.ownerDocument !== document) return null;
    let hit = document.elementFromPoint(x, y);
    // elementFromPoint stops at a shadow host; descend through
    // shadow roots to the real element the point is over, so a
    // control inside a web component is not read as "covered".
    while (hit && hit.shadowRoot) {
        const inner = hit.shadowRoot.elementFromPoint(x, y);
        if (!inner || inner === hit) break;
        hit = inner;
    }
    if (!hit || hit === this || this.contains(hit) || hit.contains(this)) return null;
    // Part of the target itself: text inside its label, or a floating label,
    // placeholder or twin input sharing a wrapper with no other control. The
    // page's body is no control's wrapper.
    if ([...(this.labels || [])].some((label) => label.contains(hit))) return null;
    let shared = hit.parentElement;
    while (shared && !shared.contains(this)) shared = shared.parentElement;
    // What is hit counts as the target's own only when it takes the text itself,
    // as a twin input does: a button over the field is a control of its own.
    const TWIN = 'textarea,[contenteditable],input:not([type=button],[type=submit],[type=reset],[type=image],[type=checkbox],[type=radio],[type=file])';
    const other = (e) => (e !== hit || !hit.matches(TWIN)) && !e.contains(this) && !this.contains(e);
    if (shared && shared !== document.body && ![...shared.querySelectorAll(CONTROLS)].some(other)) return null;
    const words = (hit.getAttribute('aria-label') || hit.innerText || '').trim().slice(0, COVER_CHARS);
    if (words) return { node: hit, detail: JSON.stringify(words) };
    // Nothing to call it by: where it is, so it can be looked at or clicked.
    const box = hit.getBoundingClientRect();
    const middle = (low, size, limit) => Math.round((Math.max(low, 0) + Math.min(low + size, limit)) / 2);
    const size = `${Math.round(box.width)}x${Math.round(box.height)}`;
    const centre = `${middle(box.left, box.width, innerWidth)},${middle(box.top, box.height, innerHeight)}`;
    return { node: hit, detail: `a ${hit.tagName.toLowerCase()} with no words on it, ${size} with its middle at ${centre}` };
}
