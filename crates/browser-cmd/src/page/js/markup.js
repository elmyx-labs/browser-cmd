function (levels) {
    // A selector is written from short attributes; a long one is a payload —
    // an inline style sheet, a base64 image — that only costs the reader.
    const ATTRIBUTE_CHARS = 200;
    let node = this;
    for (let up = 0; up < levels && node.parentElement && node.parentElement !== document.body; up++) node = node.parentElement;
    const copy = node.cloneNode(true);
    for (const gone of copy.querySelectorAll('script, style, noscript, template')) gone.remove();
    for (const drawing of copy.querySelectorAll('svg')) drawing.innerHTML = '';
    for (const element of [copy, ...copy.querySelectorAll('*')]) {
        for (const attribute of [...element.attributes]) {
            if (attribute.value.length > ATTRIBUTE_CHARS || attribute.value.startsWith('data:')) {
                element.setAttribute(attribute.name, '…');
            }
        }
    }
    return copy.outerHTML.replace(/>\s+</g, '><').replace(/\s{2,}/g, ' ');
}
