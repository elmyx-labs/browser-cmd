function (text) {
    const shapes = {
        date: 'a date as YYYY-MM-DD', time: 'a time as HH:MM', month: 'a month as YYYY-MM',
        week: 'a week as YYYY-Www', 'datetime-local': 'a date and time as YYYY-MM-DDTHH:MM',
        color: 'a colour as #rrggbb', range: `a number from ${this.min || 0} to ${this.max || 100}`,
    };
    if (this.tagName !== 'INPUT' || !(this.type in shapes)) return null;
    this.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(this, text);
    if (this.value === '' && text !== '') return shapes[this.type];
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return '';
}
