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
    const other = (e) => e !== hit && !e.contains(this) && !this.contains(e);
    if (shared && shared !== document.body && ![...shared.querySelectorAll(CONTROLS)].some(other)) return null;
    return { node: hit, detail: (hit.getAttribute('aria-label') || hit.innerText || hit.tagName).trim().slice(0, COVER_CHARS) };
}
