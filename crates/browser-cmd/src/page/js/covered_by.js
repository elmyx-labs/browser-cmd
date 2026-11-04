function(x, y) {
    // Enough of the cover to recognise and act on it; its whole
    // text is the dialog, which the state below already carries.
    const COVER_CHARS = 160;
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
    return { node: hit, detail: (hit.getAttribute('aria-label') || hit.innerText || hit.tagName).trim().slice(0, COVER_CHARS) };
}
