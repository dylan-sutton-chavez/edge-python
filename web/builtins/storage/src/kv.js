/* localStorage + sessionStorage, sync handlers; both APIs are blocking by spec so no Promise wrapping needed. */

/* JSON-array of keys (not CSV) so keys containing commas survive the round-trip. Python parses with `json.loads`. */
const keysOf = (store) => JSON.stringify(Array.from({ length: store.length }, (_, i) => store.key(i)));

export default () => ({
    local_get: (key) => localStorage.getItem(key),
    local_set: (key, value) => { localStorage.setItem(key, value); },
    local_remove: (key) => { localStorage.removeItem(key); },
    local_clear: () => { localStorage.clear(); },
    local_keys: () => keysOf(localStorage),

    session_get: (key) => sessionStorage.getItem(key),
    session_set: (key, value) => { sessionStorage.setItem(key, value); },
    session_remove: (key) => { sessionStorage.removeItem(key); },
    session_clear: () => { sessionStorage.clear(); },
    session_keys: () => keysOf(sessionStorage),
});
