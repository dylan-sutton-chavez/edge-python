---
title: "storage (web)"
description: "localStorage, sessionStorage, and IndexedDB."
---

`storage` is key-value storage and IndexedDB from scripts. It is a JavaScript module on the browser's main thread. Import it by bare name or declare it in the `system` field of `packages.json`. The native engine rejects `import storage` at compile time, see [the native engine](/reference/modules#the-native-engine).

The surface is `local_get/set/remove/clear/keys`, the same `session_*` surface, and IndexedDB (`idb_open`, `idb_put`, `idb_get`, `idb_delete`, `idb_keys`, `idb_close`). The KV handlers are synchronous, the IndexedDB handlers suspend like `fetch`. Values cross as JSON strings, encode with `json.dumps` and decode with `json.loads`. `idb_open(name, version, schema)` takes a JSON schema such as `'{"stores":["items"]}'` declaring the object stores to create on first open or a version bump.
