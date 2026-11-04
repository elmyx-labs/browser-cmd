function (how) {
  // Called on a control, or with none on whatever has focus.
  const self = this instanceof Element ? this : document.activeElement;
  if (!self) return { submits: false };
  // Enough of the button to tell one form on a page from another; the label is
  // half of the identity the repeat guard remembers, not something to read.
  const LABEL_CHARS = 40;
  const form = self.form || (self.closest ? self.closest('form') : null);
  if (!form) return { submits: false };
  const tag = self.tagName;
  const submitControl = e => e && ((e.tagName === 'INPUT' && (e.type === 'submit' || e.type === 'image'))
    || (e.tagName === 'BUTTON' && (e.type === 'submit' || !e.type)));
  let control = null;
  if (how === 'click') {
    if (!submitControl(self)) return { submits: false };
    control = self;
  } else {
    // Enter submits from a text-like field, through the form's default button
    // when it has one, or implicitly when the form has exactly one such field.
    const textLike = tag === 'INPUT' && !['checkbox','radio','button','submit','reset','file','image','hidden','range','color'].includes(self.type);
    if (!textLike) return { submits: false };
    control = form.querySelector('button[type=submit],button:not([type]),input[type=submit],input[type=image]');
    if (!control) {
      const fields = form.querySelectorAll('input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([type=button]):not([type=submit]):not([type=reset]):not([type=file]):not([type=image])');
      if (fields.length !== 1) return { submits: false };
    }
  }
  const label = control ? (control.value || control.innerText || control.textContent || '').trim().slice(0, LABEL_CHARS) : '';
  return { submits: true, id: (form.action || '') + '#' + label };
}
