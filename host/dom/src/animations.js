/* Web Animations API; `finish_msg` arrives via `receive()` on completion. */

export default ({ node, animations }, { pushEvent, emitError }) => ({
    // keyframes/options are JSON. finish_msg wakes receive() with {msg, animation_handle, ok:true}. Auto-disposes on finish or cancel.
    animate: (h, keyframesJson, optionsJson, finish_msg) => {
        const target = node(h);
        const keyframes = JSON.parse(keyframesJson);
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        /* Infinity doesn't survive JSON; accept "Infinity" as a string escape. */
        if (opts.iterations === 'Infinity') opts.iterations = Infinity;
        const anim = target.animate(keyframes, opts);
        const slot = animations.length;
        animations.push(anim);
        anim.finished
            .then(() => {
                if (finish_msg) pushEvent(JSON.stringify({ msg: finish_msg, animation_handle: slot, ok: true }));
                animations[slot] = null;
            })
            .catch((e) => {
                // AbortError on cancel is expected; surface anything else.
                if (e && e.name !== 'AbortError') emitError('animate', e);
                animations[slot] = null;
            });
        return slot;
    },

    animation_play: (h) => { if (animations[h]) animations[h].play(); },
    animation_pause: (h) => { if (animations[h]) animations[h].pause(); },
    animation_cancel: (h) => { if (animations[h]) animations[h].cancel(); },
    animation_finish: (h) => { if (animations[h]) animations[h].finish(); },
    animation_reverse: (h) => { if (animations[h]) animations[h].reverse(); },
    // For iterations:"Infinity" loops that never auto-dispose. cancel() stops the running animation before nulling.
    animation_dispose: (h) => {
        if (animations[h]) {
            animations[h].cancel();
            animations[h] = null;
        }
    },
});
