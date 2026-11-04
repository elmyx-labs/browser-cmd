function () {
  return !(this.disabled || this.readOnly) && (this.isContentEditable || 'value' in this);
}
