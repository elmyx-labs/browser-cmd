function () {
    if (typeof this.value === 'string') return this.value;
    return this.isContentEditable ? this.innerText : null;
}
