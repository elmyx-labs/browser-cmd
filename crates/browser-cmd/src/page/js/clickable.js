function (text) {
  // When a name fits several controls, the answer is the list to choose from:
  // enough of them, and enough of each, to pick one — not the whole page.
  const AMBIGUOUS_SHOWN = 8;
  const NAME_CHARS = 60;
  const want = text.trim().toLowerCase();
  const clickable = (el) => {
    const role = el.getAttribute("role");
    if (["button","link","tab","menuitem","menuitemcheckbox","menuitemradio","option","checkbox","radio","switch"].includes(role)) return true;
    if (["A","BUTTON","SELECT","SUMMARY","LABEL"].includes(el.tagName)) return true;
    if (el.onclick || el.hasAttribute("onclick")) return true;
    if (el.tabIndex >= 0) return true;
    // A control that wraps a hidden combobox input — react-select and kin —
    // has no role of its own and a default cursor, but the input inside gives
    // it away. The leaves pass below keeps only the innermost such wrapper.
    if (el.querySelector("[role=combobox], [role=listbox], [aria-haspopup=listbox]")) return true;
    try { if (getComputedStyle(el).cursor === "pointer") return true; } catch (e) {}
    return false;
  };
  // Rendered, not necessarily in view: a real box and not hidden. A target
  // below the fold has a box too, so it is a candidate — the one that wins is
  // scrolled into view before its point is read, so click-by-name reaches
  // anywhere on the page in one move, not only what is already on screen.
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return false;
    try { const s = getComputedStyle(el); if (s.visibility === "hidden" || s.display === "none") return false; } catch (e) {}
    return true;
  };
  const label = (el) => (el.getAttribute("aria-label") || el.textContent || el.value || "").trim().toLowerCase();
  // Descend open shadow roots, where design systems keep their real buttons:
  // a flat querySelectorAll stops at the custom-element host and never sees
  // the control inside it.
  const collect = (root, acc) => {
    for (const el of root.querySelectorAll("*")) {
      acc.push(el);
      if (el.shadowRoot) collect(el.shadowRoot, acc);
    }
    return acc;
  };
  const all = collect(document, []).filter((el) => shown(el) && clickable(el));
  const exact = all.filter((el) => label(el) === want);
  const pool = exact.length ? exact : all.filter((el) => label(el).includes(want) && want.length > 1);
  const leaves = pool.filter((el) => !pool.some((other) => other !== el && el.contains(other)));
  if (leaves.length === 0) return null;
  if (leaves.length > 1) {
    const names = leaves
      .slice(0, AMBIGUOUS_SHOWN)
      .map((el) => (el.getAttribute("aria-label") || el.textContent || "").trim().slice(0, NAME_CHARS));
    return { node: null, detail: JSON.stringify(names) };
  }
  return { node: leaves[0], detail: null };
}
