async function (limit) {
  // The bytes of a document the viewer draws, as base64, when they fit.
  const bytes = await fetch(location.href).then((response) => response.arrayBuffer());
  if (bytes.byteLength > limit) return { over: bytes.byteLength };
  return { data: btoa(Array.from(new Uint8Array(bytes), (byte) => String.fromCharCode(byte)).join('')) };
}
