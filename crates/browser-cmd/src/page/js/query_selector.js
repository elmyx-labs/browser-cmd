function (selector) {
    let found;
    try {
        found = document.querySelector(selector);
    } catch (error) {
        return { node: null, detail: `${JSON.stringify(selector)} is not a CSS selector` };
    }
    return found ? { node: found, detail: null } : null;
}
