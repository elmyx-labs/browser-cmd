function (index) {
  // One of the suggestions last looked at, to aim at and to name by its ref.
  const option = globalThis.__choose.options[index];
  return option && option.isConnected ? { node: option, detail: null } : null;
}
