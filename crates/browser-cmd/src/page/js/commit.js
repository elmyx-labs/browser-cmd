function (index, wanted) {
  // Whether the chosen suggestion took, and how the page shows it: the field
  // reads it with the list closed, a multi-select marks it, it shows anew by
  // the field as a chip or a value, or the widget's hidden value changed. Also
  // what the field reads, and which option a keyboard has made active.
  const c = globalThis.__choose, field = this, option = c.options[index];
  const norm = (s) => s.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
  const visible = (e) => e.isConnected && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const reads = typeof field.value === 'string' ? field.value : field.innerText;
  const open = field.getAttribute('aria-expanded') !== 'false' && c.options.some(visible);
  const shown = () => [...c.scope.querySelectorAll('*')].some((e) =>
    !c.before.has(e) && !(c.list && c.list.contains(e)) && visible(e) && norm(e.innerText) === wanted);
  const how = (!open && norm(reads).includes(wanted) && 'value')
    || (c.list && c.list.getAttribute('aria-multiselectable') === 'true' && option.getAttribute('aria-selected') === 'true' && 'selected')
    || (shown() && 'shown')
    || (c.hidden.some(([e, value]) => e.value !== value) && 'hidden')
    || null;
  const active = c.options.findIndex((o) => (o.id && field.getAttribute('aria-activedescendant') === o.id)
    || o.matches('[aria-selected=true],[class*=focus],[class*=active],[class*=highlight],[class*=selected]'));
  return { how, reads, open, focused: field.getRootNode().activeElement === field, active };
}
