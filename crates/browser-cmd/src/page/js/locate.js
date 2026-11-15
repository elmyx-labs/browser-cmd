function(wheel) {
    if (!this.isConnected) return { error: 'this ref is stale — the page replaced the element since the last snapshot; take a fresh one and use the new ref', stale: true };
    const r = this.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return { error: 'the element has no size (hidden or not yet open); open what reveals it, or take a fresh snapshot' };
    if (r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth) {
        // How far the wheel has to turn to bring it in, where a wheel over the page turns this document.
        // Somewhere in the middle third, as a hand stops the wheel: never the exact centre.
        const rest = (size) => size * (1 / 3 + Math.random() / 3);
        const across = r.left < 0 || r.right > innerWidth ? r.left + r.width / 2 - rest(innerWidth) : 0;
        const down = r.top < 0 || r.bottom > innerHeight ? r.top + r.height / 2 - rest(innerHeight) : 0;
        if (wheel && window === top) return { away: [across, down] };
        this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    }
    return { ok: true };
}
