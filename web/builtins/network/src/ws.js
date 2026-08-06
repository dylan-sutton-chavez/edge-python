/* WebSocket, push-event pattern (streaming, so `bind_event` + `receive()` like `dom`). */

export default ({ sockets, allocSocket, socket }, { pushEvent }) => ({
    /* `ws_open(url, msg)` -> socket handle. Every event (open/message/close/error) arrives via `receive()` tagged `msg`. */
    ws_open: (url, msg, protocolsJson) => {
        const protocols = protocolsJson !== undefined ? JSON.parse(protocolsJson || '[]') : undefined;
        const ws = protocols && protocols.length ? new WebSocket(url, protocols) : new WebSocket(url);

        ws.addEventListener('open', () => pushEvent(JSON.stringify({ msg, type: 'open' })));
        ws.addEventListener('message', (e) => pushEvent(JSON.stringify({
            msg, type: 'message',
            data: typeof e.data === 'string' ? e.data : undefined,
            /* binary messages surface as `binary: true`; bytes aren't crossed back yet (needs an out-of-band channel like a file handle). */
            binary: typeof e.data !== 'string' || undefined,
        })));
        ws.addEventListener('close', (e) => pushEvent(JSON.stringify({
            msg, type: 'close', code: e.code, reason: e.reason, was_clean: e.wasClean,
        })));
        ws.addEventListener('error', () => pushEvent(JSON.stringify({ msg, type: 'error' })));

        return allocSocket(ws);
    },

    /* `ws_send(h, data)`, strings only for now; binary would need a bytes handle channel. */
    ws_send: (h, data) => { socket(h).send(data); },

    /* `ws_close(h, code?, reason?)`, defaults to a clean close (1000). */
    ws_close: (h, code, reason) => {
        const ws = socket(h);
        if (code !== undefined) ws.close(code, reason || '');
        else ws.close();
        sockets[h] = null;
    },

    /* `ws_state(h)` -> 0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED. */
    ws_state: (h) => socket(h).readyState,
});
