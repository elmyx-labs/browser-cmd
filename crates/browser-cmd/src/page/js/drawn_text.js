(() => {
  // Opt-in: what 2D canvases write — labels, legends, tooltips — kept per
  // canvas in its own bitmap pixels, so a reader can place it in the view.
  // A full clear forgets it; the newest 300 strings per canvas are kept.
  const texts = new WeakMap(), proto = CanvasRenderingContext2D.prototype;
  for (const name of ['fillText', 'strokeText']) {
    const draw = proto[name];
    proto[name] = function (text, x, y, ...rest) {
      try {
        const at = this.getTransform().transformPoint(new DOMPoint(x, y));
        const list = texts.get(this.canvas) || [];
        if (list.length >= 300) list.shift();
        list.push({ text: String(text), x: at.x, y: at.y });
        texts.set(this.canvas, list);
      } catch (_) {}
      return draw.call(this, text, x, y, ...rest);
    };
  }
  const clear = proto.clearRect;
  proto.clearRect = function (x, y, w, h) {
    if (x <= 0 && y <= 0 && x + w >= this.canvas.width && y + h >= this.canvas.height) texts.delete(this.canvas);
    return clear.call(this, x, y, w, h);
  };
  Object.defineProperty(window, Symbol.for('browser-cmd.drawn'), {
    value: () => [...document.querySelectorAll('canvas')].flatMap((c) => {
      const r = c.getBoundingClientRect(), sx = r.width / (c.width || 1), sy = r.height / (c.height || 1);
      return (texts.get(c) || [])
        .map((t) => ({ text: t.text.trim(), x: Math.round(r.left + t.x * sx), y: Math.round(r.top + t.y * sy) }))
        .filter((t) => t.text && t.x >= 0 && t.y >= 0 && t.x <= innerWidth && t.y <= innerHeight);
    }),
  });
})();
