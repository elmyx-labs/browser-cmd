async function () {
  // Everything a real Chrome will say about itself when asked properly; absent
  // until the document is a secure context.
  return navigator.userAgentData
    ? navigator.userAgentData.getHighEntropyValues(['architecture', 'bitness', 'model',
        'platformVersion', 'fullVersionList', 'uaFullVersion', 'wow64', 'formFactors'])
    : null;
}
