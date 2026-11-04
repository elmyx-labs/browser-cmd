function () {
  // Selecting a contenteditable and pressing Backspace hangs the key event in a
  // headless Chrome, and typing over a selection appends there.
  const el = document.activeElement;
  if (el && el.isContentEditable) { el.textContent = ''; return true; }
  return false;
}
