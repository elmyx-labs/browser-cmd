function () {
    // How much of the DOM changed since the last look, per 100 ms, and whether
    // there is anything to read yet. The first look only starts counting.
    const g = globalThis, now = performance.now();
    const fresh = !g.__quiet;
    if (fresh) {
        g.__quiet = { at: now, changed: 0 };
        new MutationObserver((records) => {
            for (const r of records) g.__quiet.changed += r.addedNodes.length + r.removedNodes.length || 1;
        }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
    }
    const q = g.__quiet, nodes = document.getElementsByTagName('*').length || 1;
    const ratio = fresh ? null : (q.changed / nodes) * (100 / Math.max(now - q.at, 1));
    q.changed = 0;
    q.at = now;
    const body = document.body;
    const content = !!body && !!(
        body.querySelector('a[href],button,input,select,textarea,[role=button],[role=link],[tabindex],[contenteditable]')
        || body.querySelector('main,nav,header,footer,form,[role=main],[role=navigation],[role=banner],[role=search]')
        || body.innerText.trim().length >= 20);
    // How much of the view canvases cover: their redraws never touch the DOM.
    let canvas = 0;
    for (const c of document.getElementsByTagName('canvas')) {
        const r = c.getBoundingClientRect();
        canvas += Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0))
            * Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
    }
    return { ready: document.readyState, ratio, content, age: now, canvas: canvas / (innerWidth * innerHeight || 1) };
}
