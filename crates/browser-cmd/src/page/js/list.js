function (selector) {
  // Where a virtualized list's first row is, and the height of the box that
  // clips its rows: the nearest ancestor that does not overflow visible, which
  // a transform grid clips with `hidden` rather than scrolling.
  const row = document.querySelector(selector);
  if (!row) return null;
  const box = row.getBoundingClientRect();
  let height = innerHeight;
  for (let node = row.parentElement; node; node = node.parentElement) {
    if (getComputedStyle(node).overflowY !== 'visible' && node.clientHeight > 0) { height = node.clientHeight; break; }
  }
  return {
    x: Math.round(box.left + box.width / 2),
    y: Math.round(Math.min(box.top + box.height / 2, innerHeight / 2)),
    height,
  };
}
