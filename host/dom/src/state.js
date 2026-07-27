/* Shared handle tables + helpers. One `makeState()` per `createWorker` so multiple workers don't share `nodes[]`. */

const HANDLE_NONE = -1;

export const makeState = () => {
    const nodes = [];
    // Reverse lookup Element->handles for O(1) subtree sweep. WeakMap so detached refs can GC.
    const handleByEl = new WeakMap();
    const bindings = [];
    const intersectionObservers = [];
    const resizeObservers = [];
    const mutationObservers = [];
    const animations = [];
    const files = [];

    const alloc = (n) => {
        if (n === null || n === undefined) return HANDLE_NONE;
        nodes.push(n);
        const h = nodes.length - 1;
        let set = handleByEl.get(n);
        if (!set) { set = new Set(); handleByEl.set(n, set); }
        set.add(h);
        return h;
    };

    const node = (h) => {
        if (h < 0 || h >= nodes.length || nodes[h] === null) {
            throw new Error('invalid DOM node handle: ' + h);
        }
        return nodes[h];
    };

    const allocList = (list) => {
        if (!list || list.length === 0) return '';
        const out = new Array(list.length);
        for (let i = 0; i < list.length; i++) out[i] = alloc(list[i]);
        return out.join(',');
    };

    // Targets: handles in `nodes[]`, listeners in `bindings[]`, running `animations[]` rooted in `el` (inclusive). Slots nulled, not spliced.
    const cleanSubtree = (el) => {
        const all = new Set([el, ...el.querySelectorAll('*')]);
        for (const e of all) {
            const set = handleByEl.get(e);
            if (set) {
                for (const h of set) nodes[h] = null;
                handleByEl.delete(e);
            }
        }
        for (let i = 0; i < bindings.length; i++) {
            const b = bindings[i];
            if (b && all.has(b.target)) {
                b.target.removeEventListener(b.type, b.listener, { capture: b.capture });
                bindings[i] = null;
            }
        }
        for (let i = 0; i < animations.length; i++) {
            const a = animations[i];
            if (a && a.effect && all.has(a.effect.target)) {
                a.cancel();
                animations[i] = null;
            }
        }
    };

    return {
        nodes, bindings, intersectionObservers, resizeObservers, mutationObservers,
        animations, files, alloc, node, allocList, cleanSubtree,
    };
};
