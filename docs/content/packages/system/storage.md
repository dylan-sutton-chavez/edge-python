---
title: "storage (browser)"
description: "localStorage, sessionStorage, and IndexedDB."
---

`storage` is key-value storage and IndexedDB from scripts. It is a JavaScript module that needs a browser, it runs on the main thread beside `localStorage` and `indexedDB`. Declare it with `edge add storage` and import it by bare name. The CLI rejects it at compile time with `module 'storage' requires a browser`, see [The CLI](/reference/modules#the-cli). A JavaScript runtime without a page loads it and fails at the first call with `module 'storage' needs 'localStorage', missing in this runtime`, raised in the calling coroutine.

The surface is `local_get/set/remove/clear/keys`, the same `session_*` surface, and IndexedDB (`idb_open`, `idb_put`, `idb_get`, `idb_delete`, `idb_keys`, `idb_close`). The KV handlers are synchronous, the IndexedDB handlers suspend like `fetch`. Values cross as JSON strings, encode with `json.dumps` and decode with `json.loads`. `idb_open(name, version, schema)` takes a JSON schema such as `'{"stores":["items"]}'` declaring the object stores to create on first open or a version bump.
