function (limit) {
  return Array.from(document.images).slice(0, limit).map((image) => ({
    src: image.currentSrc || image.src,
    alt: (image.alt || '').trim(),
    width: image.naturalWidth || image.width || 0,
    height: image.naturalHeight || image.height || 0,
  }));
}
