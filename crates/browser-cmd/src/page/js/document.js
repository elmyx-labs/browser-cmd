function () {
  const busy = (node) => !!node && node.getAttribute('aria-busy') === 'true';
  return {
    title: document.title,
    ready: document.readyState,
    busy: busy(document.documentElement) || busy(document.body),
    contentType: document.contentType,
    text: !!document.body && document.body.innerText.trim() !== '',
    // The browser's own page for a load that failed, wherever the tab says it is.
    error: location.protocol === 'chrome-error:',
    // The server's answer, from the navigation timing entry, which costs no
    // protocol domain to read.
    status: (performance.getEntriesByType('navigation')[0] || {}).responseStatus || 0,
    // The browser's own date and zone, which is what the pages it shows see.
    clock: ((d, p) => `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
      + ` (${Intl.DateTimeFormat().resolvedOptions().timeZone})`)(new Date(), (n) => String(n).padStart(2, '0')),
  };
}
