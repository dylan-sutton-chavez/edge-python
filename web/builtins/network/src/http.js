/* HTTP async handlers; worker parks the coro on the returned Promise so Python sees `fetch()` as a yielding builtin composing with `gather` / `with_timeout`. */

export default ({ requests }) => ({
    /* `fetch(url, options_json?)` -> JSON `{id, ok, status, headers, body}`. `id` cancels via `abort_request(id)`. `options_json` forwarded as `RequestInit`. */
    fetch: async (url, optionsJson) => {
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        const ctrl = new AbortController();
        requests.push(ctrl);
        const id = requests.length - 1;
        try {
            const r = await fetch(url, { ...opts, signal: ctrl.signal });
            const headers = {};
            r.headers.forEach((v, k) => { headers[k] = v; });
            const body = await r.text();
            requests[id] = null;
            return JSON.stringify({ id, ok: r.ok, status: r.status, headers, body });
        } catch (e) {
            requests[id] = null;
            return JSON.stringify({ id, ok: false, status: 0, error: e.message });
        }
    },

    /* `fetch_text(url, options_json?)` -> body string. Raises on non-2xx via the host. */
    fetch_text: async (url, optionsJson) => {
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        const r = await fetch(url, opts);
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return await r.text();
    },

    /* `fetch_json(url, options_json?)` -> JSON body as string. Python parses with `json.loads`. */
    fetch_json: async (url, optionsJson) => {
        const opts = optionsJson !== undefined ? JSON.parse(optionsJson || '{}') : {};
        const r = await fetch(url, opts);
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return await r.text();
    },

    /* Cancels an in-flight `fetch` by handle. No-op if the request already settled. */
    abort_request: (id) => {
        const ctrl = requests[id];
        if (ctrl) {
            ctrl.abort();
            requests[id] = null;
        }
    },
});
