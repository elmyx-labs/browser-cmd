function (text) {
  // The innermost visible element whose text carries the words, brought to the
  // middle of the view.
  const want = text.trim().toLowerCase();
  const shown = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  const all = [...document.querySelectorAll('body *')].filter((el) =>
    shown(el) && el.textContent.toLowerCase().includes(want));
  const leaf = all.find((el) => !all.some((other) => other !== el && el.contains(other)));
  if (!leaf) return false;
  leaf.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  return true;
}
