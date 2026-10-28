function(wanted) {
    if (this.tagName !== 'SELECT') return { error: 'not a native select; click it, then click the option' };
    const options = Array.from(this.options);
    const lower = wanted.trim().toLowerCase();
    const match = options.find((o) => o.text.trim().toLowerCase() === lower || o.value.toLowerCase() === lower)
        || options.find((o) => o.text.toLowerCase().includes(lower));
    if (!match) return { error: 'no such option', options: options.map((o) => o.text.trim()).slice(0, 60) };
    this.focus();
    this.value = match.value;
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return { chosen: match.text.trim() };
}
