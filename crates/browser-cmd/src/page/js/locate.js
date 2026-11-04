function() {
    if (!this.isConnected) return { error: 'this ref is stale — the page replaced the element since the last snapshot; take a fresh one and use the new ref', stale: true };
    this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const r = this.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return { error: 'the element has no size (hidden or not yet open); open what reveals it, or take a fresh snapshot' };
    return { ok: true };
}
