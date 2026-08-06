/* `<video>` and `<audio>` controls. */

export default ({ node }, { emitError }) => ({
    /* play() may reject under user-gesture policies; surfaces via emitError. */
    media_play: (h) => {
        const p = node(h).play();
        if (p && p.catch) p.catch(e => emitError('media_play', e));
    },
    media_pause: (h) => { node(h).pause(); },

    get_current_time: (h) => node(h).currentTime || 0,
    set_current_time: (h, t) => { node(h).currentTime = t; },

    /* NaN until metadata loads; coerce to 0 so Python's float math stays clean. */
    get_duration: (h) => {
        const d = node(h).duration;
        return isFinite(d) ? d : 0;
    },

    get_paused: (h) => node(h).paused,

    set_volume: (h, v) => { node(h).volume = v; },
    set_playback_rate: (h, r) => { node(h).playbackRate = r; },
});
