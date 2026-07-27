# Edge Python Storage

Persistent client-side storage, `localStorage`, `sessionStorage`, `IndexedDB`. Plain ESM module registered with `createWorker({ mainThreadModules: { storage } })` (or the `host` field of `packages.json`); see [`host/README.md`](../README.md) for the setup boilerplate.

```python
from storage import local_set, local_get, idb_open, idb_put, idb_get
from "https://cdn.edgepython.com/std/json.wasm" import loads

local_set("theme", "dark")
print(local_get("theme")) # -> "dark"

db = idb_open("notes", 1, '{"stores":["items"]}')
idb_put(db, "items", "1", '{"title":"hello"}')
note = loads(idb_get(db, "items", "1"))
```

## Testing

```bash
deno run -A npm:playwright install chromium # one-time
HOSTCAP=storage deno test --allow-all tests/ # from repo root
```

See [`tests/README.md`](../tests/README.md) for the corpus shape.

## API

### Conventions

- **KV handlers are sync.** `localStorage` / `sessionStorage` are blocking by spec; handlers return strings or `None`. No `await`, no `receive()`.
- **IndexedDB handlers yield.** They return a Promise on the JS side; the runtime parks the coro in `WaitingHostCall` until resolved, same shape as `fetch()` in [`network/`](../network/README.md).
- **Values cross as JSON strings.** Encode with `json.dumps`, decode with `json.loads`.
- **Key listings are JSON arrays** (keys can contain commas). Parse with `json.loads`.
- **Handles are integer IDs** for IndexedDB; `local_*` / `session_*` address global stores directly (no handle).

### localStorage / sessionStorage

`local_get`, `local_set`, `local_remove`, `local_clear`, `local_keys`. Same surface for `sessionStorage` with the `session_` prefix (`session_get`, …). Difference is lifetime: sessionStorage clears when the tab closes; localStorage persists.

```python
local_set("theme", "dark")
print(local_get("missing")) # -> None
print(loads(local_keys())) # -> ["theme"]
local_remove("theme")
local_clear()
```

### IndexedDB

`idb_open`, `idb_put`, `idb_get`, `idb_delete`, `idb_keys`, `idb_close`.

```python
# Schema declares the object stores to create on first open / version bump.
db = idb_open("notes", 1, '{"stores":["items","tags"]}')
idb_put(db, "items", "1", dumps({"title": "hello", "ts": 1234}))
item = loads(idb_get(db, "items", "1"))
keys = loads(idb_keys(db, "items"))
idb_close(db)
```

Because IndexedDB handlers yield, they compose with the scheduler:

```python
items, tags = gather(idb_keys(db, "items"), idb_keys(db, "tags")) # parallel reads
item = with_timeout(0.5, idb_get(db, "items", "1")) # deadline
```

## How it works

`src/index.js` is a factory `() => handlers` (same shape as `dom`, `network`). Two slices in `src/` (`kv`, `idb`) close over a shared `state` (a handle table for open `IDBDatabase` instances) and merge with `Object.assign`. KV handlers call `localStorage` / `sessionStorage` directly; IDB handlers promisify native `IDBRequest`s and the runtime parks until resolved.

## License

Apache-2.0
