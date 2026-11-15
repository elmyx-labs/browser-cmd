function(wanted) {
    if (this.tagName !== 'SELECT') return { error: 'not a native select; click it, then click the option' };
    const options = Array.from(this.options);
    const lower = wanted.trim().toLowerCase();
    const match = options.find((o) => o.text.trim().toLowerCase() === lower || o.value.toLowerCase() === lower)
        || options.find((o) => o.text.toLowerCase().includes(lower));
    if (!match) return { error: 'no such option', options: options.map((o) => o.text.trim()).slice(0, 60) };
    const disabled = (o) => o.disabled || o.parentElement?.tagName === 'OPTGROUP' && o.parentElement.disabled;
    if (this.matches(':disabled')) return { error: 'the select is disabled' };
    if (disabled(match)) return { error: 'the option is disabled' };
    // Set by script, and a page can tell: its input and change events are
    // not trusted ones. The keys that would choose it open a native picker
    // on macOS that takes the keys and never answers.
    this.focus();
    this.value = match.value;
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return { chosen: match.text.trim() };
}
