import { makeState } from './state.js';
import tree from './tree.js';
import style from './style.js';
import events from './events.js';
import forms from './forms.js';
import observers from './observers.js';
import animations from './animations.js';
import media from './media.js';
import platform from './platform.js';

/* Factory consumed by `createWorker({ mainThreadModules: { dom } })`. Composes all handler slices over a shared `state`. */
export const dom = (ctx) => {
    const state = makeState();
    let errorMsg = null;
    // Surfaces async errors (listener throws, observer callbacks, swallowed promise rejections) to Python. Falls back to console.error if no consumer bound via bind_global_error.
    const emitError = (where, e) => {
        if (errorMsg) ctx.pushEvent(JSON.stringify({
            msg: errorMsg,
            where,
            error: (e && e.message) ? e.message : String(e),
            stack: (e && e.stack) ? e.stack : undefined,
        }));
        else console.error(`[dom:${where}]`, e);
    };
    const ctxPlus = { ...ctx, emitError };
    const handlers = Object.assign(
        {},
        tree(state),
        style(state),
        events(state, ctxPlus),
        forms(state, ctxPlus),
        observers(state, ctxPlus),
        animations(state, ctxPlus),
        media(state, ctxPlus),
        platform(state, ctxPlus),
        { bind_global_error: (msg) => { errorMsg = msg; } },
    );

    /* Mutators an `apply_batch` op list may call, anything returning a value cannot batch. */
    const BATCHABLE = new Set([
        'set_text', 'set_html', 'set_attribute', 'remove_attribute',
        'add_class', 'remove_class', 'set_data',
        'append_child', 'insert_before', 'remove', 'replace_children',
        'set_style', 'set_scroll_top', 'scroll_into_view', 'focus', 'blur',
        'set_value', 'set_checked',
    ]);

    /* One call, many mutations, `ops` is a list of `[name, ...args]`. See the `dom_batch` package for the Python side. */
    handlers.apply_batch = (ops) => {
        for (const [name, ...args] of ops) {
            if (!BATCHABLE.has(name)) throw new Error(`apply_batch: '${name}' is not batchable`);
            handlers[name](...args);
        }
    };
    return handlers;
};

export default dom;
