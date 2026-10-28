function (values) {
    // Where each string shows in the viewport now, from the rendered text of
    // the document: a fact about what a person would see, not what it means.
    const MATCHES = 200;
    const norm = (s) => s.replace(/\s+/g, ' ').trim().toLowerCase();
    // The document's text with runs of white space made one space and every
    // character lowered, and for each character the text node and offset it
    // came from, so a match that spans elements still finds its box.
    const chars = [], from = [];
    const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT, {
        acceptNode: (node) => /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE)$/.test(node.parentElement?.tagName)
            ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
    });
    let space = true;
    for (let node; (node = walker.nextNode());) {
        const data = node.data;
        for (let i = 0; i < data.length; i++) {
            if (/\s/.test(data[i])) {
                if (!space) { chars.push(' '); from.push([node, i]); }
                space = true;
                continue;
            }
            for (const c of data[i].toLowerCase()) { chars.push(c); from.push([node, i]); }
            space = false;
        }
    }
    const text = chars.join('');
    return values.map((value) => {
        const want = norm(value);
        const found = { value, total: 0, visible: 0, first: null, nearest: null, fields: 0 };
        if (!want) return found;
        for (let i = text.indexOf(want); i >= 0 && found.total < MATCHES; i = text.indexOf(want, i + 1)) {
            found.total++;
            const [start, startAt] = from[i], [end, endAt] = from[i + want.length - 1];
            const range = document.createRange();
            range.setStart(start, startAt);
            range.setEnd(end, endAt + 1);
            const box = [...range.getClientRects()].find((r) => r.width > 0 && r.height > 0);
            if (!box) continue;
            const element = start.parentElement;
            const where = {
                x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2),
                pageY: Math.round(box.top + scrollY), tag: element.tagName.toLowerCase(),
            };
            const inView = box.bottom > 0 && box.right > 0 && box.top < innerHeight && box.left < innerWidth
                && element.checkVisibility({ opacityProperty: true, visibilityProperty: true });
            if (inView) {
                found.visible++;
                found.first ??= where;
            } else if (!found.nearest || Math.abs(where.y) < Math.abs(found.nearest.y)) {
                found.nearest = where;
            }
        }
        found.fields = [...document.querySelectorAll('input, textarea')]
            .filter((field) => norm(field.value).includes(want)).length;
        return found;
    });
}
