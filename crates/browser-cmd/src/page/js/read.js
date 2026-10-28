function (links, context, skip) {
  // Called on an element, or with none on the whole document.
  const self = this instanceof Element ? this : document.body;
  if (!self) return null;
  if (!links && !context) return self.innerText;
  const block = /^(P|DIV|LI|TR|SECTION|ARTICLE|UL|OL|H[1-6]|HEADER|FOOTER|NAV|MAIN|ASIDE|BR|HR|BLOCKQUOTE|PRE|TABLE|FIGURE)$/;
  const parts = [];
  const inline = (n) => {
    const label = (n.innerText || n.textContent || '').trim().replace(/\s+/g, ' ');
    return label ? '[' + label + '](' + n.href + ')' : n.href;
  };
  // The element read may itself be the link, and a link read as a link is
  // the whole of what was asked for.
  if (links && self.tagName === 'A' && self.href) return inline(self);
  const tidy = s => (s || '').trim().replace(/\s+/g, ' ');
  const labelText = node => {
    if (node.nodeType === 3) return node.textContent;
    if (node.nodeType !== 1 || /^(INPUT|SELECT|TEXTAREA|BUTTON|SCRIPT|STYLE)$/.test(node.tagName)) return '';
    const style = getComputedStyle(node);
    if (style.display === 'none' || style.visibility === 'hidden') return '';
    return Array.from(node.childNodes).map(labelText).join('');
  };
  const label = n => tidy(n.getAttribute('aria-label') ||
    (n.getAttribute('aria-labelledby') || '').split(/\s+/).map(id => document.getElementById(id)?.innerText || '').join(' ') ||
    Array.from(n.labels || []).map(labelText).join(' ') || n.name);
  // A site's header, menus and footer repeat on every page, and a cookie
  // notice is not the page: with skip they are named in one line, which says
  // how to read them, and a notice keeps its title and buttons to answer it.
  const chrome = {banner: 'site header', navigation: 'navigation', contentinfo: 'footer'};
  const skipped = new Set();
  const landmark = n => {
    const kind = chrome[n.getAttribute('role') || {HEADER: 'banner', NAV: 'navigation', FOOTER: 'contentinfo'}[n.tagName]];
    return kind && !n.querySelector('h1') && !n.parentElement?.closest('main,article,[role=main],[role=article]') && kind;
  };
  const notice = (n, style) => (n.tagName === 'DIALOG' || /^(dialog|alertdialog)$/.test(n.getAttribute('role')) || /^(fixed|sticky)$/.test(style.position)) &&
    /cookie|consent|gdpr|onetrust|cookiebot|didomi|usercentrics|truste/i.test([n.id, n.getAttribute('class'), n.getAttribute('aria-label')].join(' '));
  const answers = n => {
    const title = tidy(n.getAttribute('aria-label') || n.querySelector('h1,h2,h3,h4,[role=heading]')?.innerText).slice(0, 80);
    const buttons = Array.from(n.querySelectorAll('button,[role=button],input[type=button],input[type=submit]'))
      .map(b => tidy(b.innerText || b.value)).filter(Boolean).slice(0, 6);
    return 'a cookie notice' + (title ? ' "' + title + '"' : '') + (buttons.length ? ' (buttons: ' + buttons.join(' · ') + ')' : '');
  };
  const walk = (node) => {
    for (const n of node.childNodes) {
      if (n.nodeType === 3) { parts.push(n.textContent); continue; }
      if (n.nodeType !== 1) continue;
      const tag = n.tagName;
      if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE') continue;
      let style; try { style = getComputedStyle(n); } catch (e) {}
      if (style && (style.display === 'none' || style.visibility === 'hidden')) continue;
      if (context && style && style.contentVisibility === 'hidden') continue;
      if (context && style && style.display !== 'contents' && !n.getClientRects().length) continue;
      const part = skip && style && (landmark(n) || (notice(n, style) && answers(n)));
      if (part) { skipped.add(part); continue; }
      const structural = block.test(tag) || (context && style && /^(block|flex|grid|list-item)/.test(style.display));
      if (context && tag === 'DETAILS' && !n.open) {
        const summary = n.querySelector(':scope > summary');
        if (summary) walk({childNodes: [summary]});
        continue;
      }
      if (context && (/^H[1-6]$/.test(tag) || n.getAttribute('role') === 'heading')) {
        const level = Math.min(6, Math.max(1, Number(n.getAttribute('aria-level') || tag.slice(1)) || 2));
        parts.push('\n' + '#'.repeat(level) + ' ' + tidy(n.innerText) + '\n'); continue;
      }
      if (context && tag === 'SELECT') {
        parts.push('\n[Selected: ' + (label(n) || 'select') + ' = ' + Array.from(n.selectedOptions).map(o => tidy(o.text)).join(', ') + ']\n'); continue;
      }
      if (context && tag === 'INPUT' && ['checkbox','radio'].includes(n.type)) {
        if (n.checked) parts.push('\n[Selected: ' + (label(n) || n.type) + ']\n');
        continue;
      }
      if (context && tag === 'INPUT' && !['password','hidden','file','button','submit','reset','image'].includes(n.type)) {
        if (n.value) parts.push('\n[Control value: ' + (label(n) || n.type) + ' = ' + tidy(n.value) + ']\n');
        continue;
      }
      if (context && (n.getAttribute('aria-selected') === 'true' || n.getAttribute('aria-checked') === 'true')) {
        parts.push('\n[Selected: ' + (label(n) || tidy(n.innerText)) + ']\n');
      }
      if (context && tag === 'TABLE') {
        const headers = Array.from(n.rows?.[0]?.cells || []).filter(c => c.tagName === 'TH' && c.getClientRects().length && getComputedStyle(c).visibility !== 'hidden').map(c => tidy(c.innerText));
        parts.push(headers.length ? '\n[Table columns: ' + headers.join(' | ') + ']\n' : '\n[Table]\n');
        walk(n); parts.push('\n[End table]\n'); continue;
      }
      if (context && tag === 'TR') {
        const cells = Array.from(n.cells).filter(c => c.getClientRects().length && getComputedStyle(c).visibility !== 'hidden').map(c => {
          const start = parts.length;
          walk(c);
          return tidy(parts.splice(start).join(''));
        });
        parts.push(cells.join(' | ') + '\n'); continue;
      }
      if (tag === 'A' && n.href && links) { parts.push(inline(n)); continue; }
      if (context && structural) parts.push('\n');
      walk(n);
      if (structural) parts.push('\n');
    }
  };
  walk(context ? {childNodes: [self]} : self);
  const text = parts.join('').replace(/[ \t]+/g, ' ').replace(/ ?\n ?/g, '\n').replace(/\n{2,}/g, '\n').trim();
  return skipped.size ? '[Skipped ' + Array.from(skipped).join(', ') + ' — ' + skip + ']\n' + text : text;
}
