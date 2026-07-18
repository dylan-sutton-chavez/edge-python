# Edge Python Host

Official JS modules for [Edge Python](https://edgepython.com) exposing host APIs (DOM, network, storage and more) to Python scripts. Each capability is a plain ESM registered with `createWorker` via `mainThreadModules`, no `.wasm`, no Rust, no custom embedder.

## Layout

```
├── dom
│   ├── packages.json
│   └── src
│       ├── entry.py
│       ├── index.js
│       └── main
├── network
│   └── src
│       ├── index.js
│       └── main
├── storage
│   └── src
│       ├── index.js
│       └── main
├── time
│   └── src
│       ├── index.js
│       └── main
└── tests
```

One folder per capability. Each ships a `<name>/<name>.json` corpus; the shared runner in `tests/` walks for them and drives every case through headless Chromium.

## Usage

```html
<script type="module">
    import { createWorker } from "https://cdn.edgepython.com/runtime/src/index.js";
    import { dom } from "./dom/src/index.js";

    const worker = await createWorker({
        wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
        // JS handlers register as `_dom`; scripts import the `dom` façade (see dom/packages.json).
        mainThreadModules: { _dom: dom },
        imports: { dom: "https://cdn.edgepython.com/host/dom/src/entry.py" },
    });
    await worker.run(await (await fetch("./script.py")).text());
</script>
```

## Packages

| Folder | Description |
|--------|-------------|
| `dom`     | Browser DOM access with opt-in `batch()` mutation batching, see [`dom/README.md`](dom/README.md) |
| `network` | HTTP fetch, WebSocket, SEE, see [`network/README.md`](network/README.md) |
| `storage` | localStorage, sessionStorage, IndexedDB, see [`storage/README.md`](storage/README.md) |
| `time`    | Clocks, sleep, calendar formatting, see [`time/README.md`](time/README.md) |

## Development

Requires [Deno v2](https://deno.com/) and Playwright's Chromium.

```bash
# Lint one capability
cd host/<capability> && deno lint src/

# Install Chromium (once)
deno run -A npm:playwright install --with-deps chromium

# Test one capability (from host/)
cd host && HOSTCAP=<capability> deno test --allow-all tests/
```

`<capability>` is one of `dom`, `network`, `storage`, `time`.

## License

MIT OR Apache-2.0
