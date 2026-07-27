/* Server-Sent Events, push-event pattern (one-way streaming, native browser `EventSource`). */

export default ({ sseSources, allocSse, sse }, { pushEvent }) => ({
    /* `sse_open(url, msg, options_json?)` -> handle. `options_json` accepts `{ withCredentials: bool }`. Every event (open/message/error) arrives via `receive()` tagged `msg`. */
    sse_open: (url, msg, optionsJson) => {
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        const es = new EventSource(url, opts);

        es.addEventListener('open', () => pushEvent(JSON.stringify({ msg, type: 'open' })));
        es.addEventListener('message', (e) => pushEvent(JSON.stringify({
            msg, type: 'message',
            data: e.data,
            event_id: e.lastEventId || undefined,
        })));
        es.addEventListener('error', () => pushEvent(JSON.stringify({
            msg, type: 'error', state: es.readyState,
        })));

        return allocSse(es);
    },

    /* `sse_close(h)`, terminates the connection. */
    sse_close: (h) => { sse(h).close(); sseSources[h] = null; },

    /* `sse_state(h)` -> 0=CONNECTING, 1=OPEN, 2=CLOSED. */
    sse_state: (h) => sse(h).readyState,
});
