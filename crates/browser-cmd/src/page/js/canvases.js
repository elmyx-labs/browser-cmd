function () {
  // Canvases a person can see in the viewport, largest first, with the share
  // of the viewport they cover: what is drawn there is in no tree or text.
  const seen = [...document.querySelectorAll('canvas')]
    .map(c => ({ c, r: c.getBoundingClientRect() }))
    .map(({ c, r }) => {
      const w = Math.max(0, Math.min(r.right, innerWidth) - Math.max(r.left, 0));
      const h = Math.max(0, Math.min(r.bottom, innerHeight) - Math.max(r.top, 0));
      const style = getComputedStyle(c);
      const shown = style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) > 0;
      return { c, r, area: shown ? w * h : 0 };
    })
    .filter(x => x.area >= 120 * 80)
    .sort((a, b) => b.area - a.area);
  return {
    coverage: Math.min(1, seen.reduce((sum, x) => sum + x.area, 0) / (innerWidth * innerHeight)),
    canvases: seen.slice(0, 3).map(({ c, r }) => ({
      selector: c.id ? `#${CSS.escape(c.id)}` : null,
      x: Math.round(r.left), y: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height),
    })),
  };
}
