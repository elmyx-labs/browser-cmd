function (spec) {
  const value = (node, part) => {
    if (!node) return null;
    const at = part.indexOf("@");
    const attribute = at < 0 ? null : part.slice(at + 1);
    if (!attribute) return (node.innerText || node.textContent || "").trim();
    if (attribute === "text") return (node.innerText || node.textContent || "").trim();
    if (attribute === "value") return node.value ?? null;
    if (attribute === "href" || attribute === "src") return node[attribute] ?? null;
    return node.getAttribute(attribute);
  };
  const pick = (root, fields) => {
    const out = Object.create(null);
    for (const [name, part] of Object.entries(fields)) {
      const at = part.indexOf("@");
      let selector = (at < 0 ? part : part.slice(0, at)).trim();
      // A leading "+" reaches the row's next sibling, for data a page splits
      // across sibling elements (a title row and its detail row, a dt and its
      // dd) with no element wrapping the pair.
      let base = root;
      while (base && selector.startsWith("+")) {
        base = base.nextElementSibling;
        selector = selector.slice(1).trim();
      }
      let node = null;
      if (base) {
        node = selector === "" || selector === "."
          ? base
          : base.matches?.(selector) ? base : base.querySelector(selector);
      }
      out[name] = value(node, part);
    }
    return out;
  };
  if (!spec.each) return { rows: [pick(document, spec.fields)], found: 1 };
  const all = Array.from(document.querySelectorAll(spec.each));
  const selected = all.slice(0, spec.limit);
  const rows = selected.map((row) => pick(row, spec.fields));
  // Without an identity a row is told apart by its field values.
  return { rows, found: all.length, identities: selected.map((row, at) =>
    spec.identity ? pick(row, {key: spec.identity}).key : JSON.stringify(rows[at])) };
}
