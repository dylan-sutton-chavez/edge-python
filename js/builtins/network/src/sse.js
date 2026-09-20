const CONNECTING = 0;
const OPEN = 1;
const CLOSED = 2;

// The wait before a reconnect until the stream sets its own with `retry`.
const RETRY_MS = 3000;

/* Event stream parser per the WHATWG rules, `dispatch` gets each complete event. */
const parser = (dispatch, setRetry, source) => {
    let buffer = '';
    let data = '';
    let type = '';

    const field = (name, value) => {
        if (name === 'data') data += value + '\n';
        else if (name === 'event') type = value;
        else if (name === 'id' && !value.includes('\0')) source.idBuffer = value;
        else if (name === 'retry' && /^\d+$/.test(value)) setRetry(Number(value));
    };

    const line = (text) => {
        if (text === '') {
            source.lastEventId = source.idBuffer;
            if (data !== '') dispatch(type || 'message', data.slice(0, -1));
            data = '';
            type = '';
            return;
        }
        if (text.startsWith(':')) return;
        const colon = text.indexOf(':');
        if (colon === -1) return field(text, '');
        const value = text.slice(colon + 1);
        field(text.slice(0, colon), value.startsWith(' ') ? value.slice(1) : value);
    };

    /* Splits on CRLF, LF or CR, holding a trailing CR until the next chunk shows its LF. */
    return (text, done) => {
        buffer += text;
        let start = 0;
        for (let i = 0; i < buffer.length; i++) {
            const c = buffer[i];
            if (c !== '\n' && c !== '\r') continue;
            if (c === '\r' && i + 1 === buffer.length && !done) break;
            line(buffer.slice(start, i));
            if (c === '\r' && buffer[i + 1] === '\n') i++;
            start = i + 1;
        }
        buffer = buffer.slice(start);
        // A stream that ends mid event discards the unfinished line and event.
        if (done) {
            buffer = '';
            data = '';
            type = '';
        }
    };
};

/* One event source over fetch, reconnecting the way `EventSource` does until closed. */
const connect = (url, msg, credentials, pushEvent) => {
    const source = { readyState: CONNECTING, lastEventId: '', idBuffer: '', close: () => {} };
    let retryMs = RETRY_MS;
    let ctrl = null;
    let timer = null;

    const emit = (event) => pushEvent(JSON.stringify({ msg, ...event }));

    // Only unnamed events reach `receive()`, as with the `message` listener of `EventSource`.
    const dispatch = (type, data) => {
        if (source.readyState === OPEN && type === 'message') {
            emit({ type: 'message', data, event_id: source.lastEventId || undefined });
        }
    };

    const fail = () => {
        source.readyState = CLOSED;
        emit({ type: 'error', state: CLOSED });
    };

    const reconnect = () => {
        if (source.readyState === CLOSED) return;
        source.readyState = CONNECTING;
        emit({ type: 'error', state: CONNECTING });
        timer = setTimeout(run, retryMs);
    };

    const run = async () => {
        ctrl = new AbortController();
        const headers = { Accept: 'text/event-stream' };
        if (source.lastEventId) headers['Last-Event-ID'] = source.lastEventId;
        let response;
        try {
            response = await fetch(url, { headers, credentials, signal: ctrl.signal });
        } catch {
            return reconnect();
        }
        if (source.readyState === CLOSED) return;
        const mime = (response.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase();
        if (response.status !== 200 || mime !== 'text/event-stream' || !response.body) {
            response.body?.cancel().catch(() => {});
            return fail();
        }
        source.readyState = OPEN;
        emit({ type: 'open' });
        const feed = parser(dispatch, (ms) => { retryMs = ms; }, source);
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        try {
            for (;;) {
                const { value, done } = await reader.read();
                if (done) {
                    feed(decoder.decode(), true);
                    break;
                }
                feed(decoder.decode(value, { stream: true }), false);
            }
        } catch {
            // An abort or a dropped connection both end the read, the state decides what follows.
        }
        reconnect();
    };

    source.close = () => {
        source.readyState = CLOSED;
        clearTimeout(timer);
        ctrl?.abort();
    };

    run();
    return source;
};

export default ({ sseSources, allocSse, sse }, { pushEvent }) => ({
    /* `sse_open(url, msg, options_json?)` -> handle. `options_json` accepts `{ withCredentials: bool }`. Every event (open/message/error) arrives via `receive()` tagged `msg`. */
    sse_open: (url, msg, optionsJson) => {
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        return allocSse(connect(url, msg, opts.withCredentials ? 'include' : 'same-origin', pushEvent));
    },

    /* `sse_close(h)`, terminates the connection. */
    sse_close: (h) => { sse(h).close(); sseSources[h] = null; },

    /* `sse_state(h)` -> 0=CONNECTING, 1=OPEN, 2=CLOSED. */
    sse_state: (h) => sse(h).readyState,
});
