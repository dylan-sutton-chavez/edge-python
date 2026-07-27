/* CSSOM, layout queries, focus. */

export default ({ node }) => ({
    set_style: (h, prop, value) => { node(h).style[prop] = value; },
    get_style: (h, prop) => node(h).style[prop] || '',
    get_computed_style: (h, prop) => getComputedStyle(node(h))[prop] || '',

    /* JSON {x, y, w, h, top, right, bottom, left}. */
    rect: (h) => {
        const r = node(h).getBoundingClientRect();
        return JSON.stringify({
            x: r.x, y: r.y, w: r.width, h: r.height,
            top: r.top, right: r.right, bottom: r.bottom, left: r.left,
        });
    },

    offset_width: (h) => node(h).offsetWidth,
    offset_height: (h) => node(h).offsetHeight,
    client_width: (h) => node(h).clientWidth,
    client_height: (h) => node(h).clientHeight,

    scroll_top: (h) => node(h).scrollTop,
    set_scroll_top: (h, v) => { node(h).scrollTop = v; },
    scroll_into_view: (h) => { node(h).scrollIntoView(); },

    focus: (h) => { node(h).focus(); },
    blur: (h) => { node(h).blur(); },
});
