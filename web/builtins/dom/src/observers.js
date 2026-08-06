/* Intersection / Resize / Mutation observers; each entry fires through `ctx.pushEvent`. */

export default ({ node, intersectionObservers, resizeObservers, mutationObservers }, { pushEvent, emitError }) => ({
    /* Options: {root_handle?, rootMargin?, threshold?}. Event detail: {msg, target_handle, intersecting, ratio, x, y, w, h}. */
    observe_intersection: (h, msg, optionsJson) => {
        const target = node(h);
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        if (opts.root_handle !== undefined) {
            opts.root = node(opts.root_handle);
            delete opts.root_handle;
        }
        const observer = new IntersectionObserver((entries) => {
            try {
                for (const e of entries) {
                    const r = e.boundingClientRect;
                    pushEvent(JSON.stringify({
                        msg,
                        target_handle: h,
                        intersecting: e.isIntersecting,
                        ratio: e.intersectionRatio,
                        x: r.x, y: r.y, w: r.width, h: r.height,
                    }));
                }
            } catch (err) {
                emitError('observe_intersection', err);
            }
        }, opts);
        observer.observe(target);
        intersectionObservers.push(observer);
        return intersectionObservers.length - 1;
    },

    unobserve_intersection: (h) => {
        const o = intersectionObservers[h];
        if (!o) return;
        o.disconnect();
        intersectionObservers[h] = null;
    },

    /* Fires when target's box changes (any layout reflow), not just on window resize. */
    observe_resize: (h, msg) => {
        const target = node(h);
        const observer = new ResizeObserver((entries) => {
            try {
                for (const e of entries) {
                    const r = e.contentRect;
                    pushEvent(JSON.stringify({ msg, target_handle: h, w: r.width, h: r.height, x: r.x, y: r.y }));
                }
            } catch (err) {
                emitError('observe_resize', err);
            }
        });
        observer.observe(target);
        resizeObservers.push(observer);
        return resizeObservers.length - 1;
    },

    unobserve_resize: (h) => {
        const o = resizeObservers[h];
        if (!o) return;
        o.disconnect();
        resizeObservers[h] = null;
    },

    /* Options follow MutationObserverInit. `target_handle` is the watched root; mutations may fire on descendants (see `target_id`/`target_tag`). Added/removed report counts only; re-query for the actual new nodes. */
    observe_mutations: (h, msg, optionsJson) => {
        const target = node(h);
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        /* Spec requires at least one of childList/attributes/characterData; default to the most common. */
        if (!opts.childList && !opts.attributes && !opts.characterData) opts.childList = true;
        const observer = new MutationObserver((mutations) => {
            try {
                for (const m of mutations) {
                    pushEvent(JSON.stringify({
                        msg,
                        target_handle: h,
                        type: m.type,
                        target_tag: m.target && m.target.tagName ? m.target.tagName.toLowerCase() : undefined,
                        target_id: m.target && m.target.id ? m.target.id : undefined,
                        attribute_name: m.attributeName || undefined,
                        attribute_old: m.oldValue || undefined,
                        added: m.addedNodes.length,
                        removed: m.removedNodes.length,
                    }));
                }
            } catch (err) {
                emitError('observe_mutations', err);
            }
        });
        observer.observe(target, opts);
        mutationObservers.push(observer);
        return mutationObservers.length - 1;
    },

    unobserve_mutations: (h) => {
        const o = mutationObservers[h];
        if (!o) return;
        o.disconnect();
        mutationObservers[h] = null;
    },
});
