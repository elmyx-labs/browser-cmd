function () {
  const gl = document.createElement('canvas').getContext('webgl');
  const debug = gl && gl.getExtension('WEBGL_debug_renderer_info');
  // null: no WebGL at all, so maps and 3D views draw blank.
  return gl ? (debug ? gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) : '') : null;
}
