function (phase, words) {
  // Called on a field. 'start', before typing: watch the page and remember what
  // is already on it. 'look', after: the suggestions now shown under the field.
  const OPTIONS = 20;
  const OPTION_CHARS = 200;
  // Lists of suggestion widgets that give their options no role.
  const KNOWN = '.pac-item,.select2-results__option,.chosen-results li,.tt-suggestion,.ui-menu-item,.autocomplete-suggestion';
  const g = globalThis, field = this, now = performance.now();
  // Words as they are compared: no accents, no case, no punctuation.
  const norm = (s) => s.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
  const visible = (e) => {
    const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden';
  };
  // The words of an option as they read: inline pieces of one word run
  // together, separately laid-out parts are separate words.
  const textOf = (e) => {
    if (!e.getClientRects().length) return '';
    const mixed = [...e.childNodes].some((n) => n.nodeType === 3 && n.data.trim());
    const text = mixed || !e.children.length ? e.innerText : [...e.children].map(textOf).filter(Boolean).join(' ');
    return text.replace(/\s+/g, ' ').trim();
  };
  const roots = () => {
    const all = [document];
    for (let i = 0; i < all.length; i++) for (const e of all[i].querySelectorAll('*')) if (e.shadowRoot) all.push(e.shadowRoot);
    return all;
  };
  const distance = (group) => {
    const a = field.getBoundingClientRect(), b = group[0].getBoundingClientRect();
    return Math.abs(b.top - a.bottom) + Math.abs(b.left - a.left);
  };
  const find = () => {
    const root = field.getRootNode();
    const byId = (id) => (root.getElementById && root.getElementById(id)) || document.getElementById(id);
    const owned = `${field.getAttribute('aria-controls') || ''} ${field.getAttribute('aria-owns') || ''}`.split(/\s+/).filter(Boolean).map(byId);
    const active = field.getAttribute('aria-activedescendant') && byId(field.getAttribute('aria-activedescendant'));
    if (active) owned.push(active.closest('[role=listbox],[role=menu]') || active.parentElement);
    for (const box of owned) {
      const options = box ? [...box.querySelectorAll('[role=option],[role=menuitem]')].filter(visible) : [];
      if (options.length) return options;
    }
    const groups = new Map();
    const add = (e) => {
      const box = e.closest('[role=listbox]') || e.parentElement;
      groups.set(box, [...(groups.get(box) || []), e]);
    };
    for (const r of roots()) for (const e of r.querySelectorAll(`[role=listbox] [role=option],${KNOWN}`)) if (visible(e)) add(e);
    // Else rows that appeared since typing began: repeated siblings of short text.
    if (!groups.size) {
      for (const e of document.querySelectorAll('body *')) {
        const text = !g.__choose.before.has(e) && !e.contains(field) && visible(e) && e.innerText.trim();
        if (text && text.length <= OPTION_CHARS) add(e);
      }
      for (const [box, group] of groups) if (group.length < 2 || group.some((e) => e.tagName !== group[0].tagName)) groups.delete(box);
      for (const [box] of groups) if ([...groups.values()].some((group) => group.some((e) => e.contains(box)))) groups.delete(box);
    }
    return [...groups.values()].sort((a, b) => distance(a) - distance(b))[0] || [];
  };
  if (phase === 'start') {
    if (field.tagName === 'SELECT') return { select: true };
    if (g.__choose) g.__choose.observer.disconnect();
    const c = g.__choose = { last: now, options: [], before: new WeakSet(document.querySelectorAll('*')) };
    // The field's wrapper, where a widget keeps its chips and hidden value.
    c.scope = field;
    for (let i = 0; i < 3 && c.scope.parentElement; i++) c.scope = c.scope.parentElement;
    c.observer = new MutationObserver(() => { c.last = performance.now(); });
    for (const r of roots()) c.observer.observe(r, { subtree: true, childList: true, attributes: true, characterData: true });
    c.baseline = find().map(textOf).join('\n');
    const listed = field.list && [...field.list.options].map((o) => ({ text: o.value, key: norm(o.value), disabled: o.disabled }));
    return { words: words.map(norm), datalist: listed || null };
  }
  const c = g.__choose;
  if (c.typed === undefined) {
    c.typed = now;
    c.hidden = [...c.scope.querySelectorAll('input[type=hidden]')].map((e) => [e, e.value]);
  }
  c.options = find().slice(0, OPTIONS);
  c.list = c.options.length ? c.options[0].closest('[role=listbox],[role=menu]') || c.options[0].parentElement : null;
  const options = c.options.map((e) => {
    const text = textOf(e).slice(0, OPTION_CHARS);
    return { text, key: norm(text), disabled: e.getAttribute('aria-disabled') === 'true' || e.matches('[disabled]') };
  });
  return { options, quiet: now - c.last, fresh: c.last > c.typed || options.map((o) => o.text).join('\n') !== c.baseline };
}
