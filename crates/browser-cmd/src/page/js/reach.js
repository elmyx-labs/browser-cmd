function (x, y) {
  // Sub-pixel layout leaves a fraction between scrollHeight and clientHeight on
  // boxes that do not scroll at all; a whole pixel of slack ignores it.
  const ROUNDING = 1;
  const doc = document.scrollingElement || document.documentElement;
  const of = (node, kind) => ({
    kind,
    top: Math.round(node.scrollTop),
    max: Math.round(node.scrollHeight - node.clientHeight),
    client: Math.round(node.clientHeight),
    left: Math.round(node.scrollLeft),
    maxLeft: Math.round(node.scrollWidth - node.clientWidth),
  });
  if (x === null || y === null) return of(doc, 'page');
  let hit = document.elementFromPoint(x, y);
  // Descend shadow roots so a scroll region inside a web component is found,
  // not the host that stands in front of it.
  while (hit && hit.shadowRoot) {
    const inner = hit.shadowRoot.elementFromPoint(x, y);
    if (!inner || inner === hit) break;
    hit = inner;
  }
  if (!hit) return { kind: 'nothing' };
  if (hit.tagName === 'IFRAME' || hit.tagName === 'FRAME') return { kind: 'frame' };
  for (let node = hit; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    const scrolls = /auto|scroll|overlay/.test(style.overflowY + ' ' + style.overflowX);
    if (scrolls
        && (node.scrollHeight > node.clientHeight + ROUNDING
            || node.scrollWidth > node.clientWidth + ROUNDING)) {
      return of(node, 'region');
    }
  }
  return of(doc, 'page');
}
