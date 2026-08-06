/* `<dialog>`, fullscreen, pointer lock, SVG geometry. */

export default ({ alloc, node }, { emitError }) => ({
    /* Native modal with backdrop + focus trap. Bind "close" event to detect dismissal. */
    show_modal: (h) => { node(h).showModal(); },
    dialog_close: (h, returnValue) => {
        const n = node(h);
        if (returnValue !== undefined) n.close(returnValue); else n.close();
    },

    /* May reject without a user gesture; surfaces via emitError. Bind "fullscreenchange" on documentElement for state. */
    request_fullscreen: (h) => {
        const p = node(h).requestFullscreen();
        if (p && p.catch) p.catch(e => emitError('request_fullscreen', e));
    },
    exit_fullscreen: () => {
        const p = document.exitFullscreen();
        if (p && p.catch) p.catch(e => emitError('exit_fullscreen', e));
    },
    fullscreen_element: () => alloc(document.fullscreenElement),

    /* While locked, clientX/Y freezes but movementX/Y keeps firing (see events.js payload). */
    request_pointer_lock: (h) => {
        const p = node(h).requestPointerLock();
        if (p && p.catch) p.catch(e => emitError('request_pointer_lock', e));
    },
    exit_pointer_lock: () => { document.exitPointerLock(); },

    /* User-space units (different from `rect()` which returns viewport pixels). */
    bbox: (h) => {
        const r = node(h).getBBox();
        return JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height });
    },

    /* Total length of an SVGPathElement; needed for stroke-dasharray "drawing" animations. */
    path_length: (h) => node(h).getTotalLength(),

    /* {x, y} at distance `dist` along the path; animating an object along a curve. */
    point_at_length: (h, dist) => {
        const p = node(h).getPointAtLength(dist);
        return JSON.stringify({ x: p.x, y: p.y });
    },
});
