function () {
  return {
    width: innerWidth,
    height: innerHeight,
    ratio: devicePixelRatio,
    left: visualViewport.pageLeft,
    top: visualViewport.pageTop,
  };
}
