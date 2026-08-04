# Tests

Shared test harness for every host capability, modeled on [`edge-python/runtime/tests`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/runtime/tests). `index.html` is pure markup: it loads the runtime's `<edge-python>` tag and nothing else. `host.test.js` boots one tag per capability and drives it through the tag's programmatic API (`run`, `onOutput`).

## Layout

```
├── host.test.js
└── index.html
```

The `<cap>/` folders and their `<cap>.json` corpora stay agnostic to testing. The driver synthesizes the `packages.json` and prepends `from <cap> import *\n`, so a corpus only holds the code under test.

## How it works

The driver serves the repo over `http://localhost` (a secure context, so the runtime's integrity check works), synthesizes a `packages.json` (the capability's `src/index.js`), and boots one `<edge-python packages="...">`. After its `ready` event it captures stdout with `el.worker.onOutput(...)` and runs each snippet with `el.worker.run(src)`, reading the trace for error cases. One worker is reused for the whole corpus, so the whole real system (runtime + `compiler.wasm` + the capability module) is exercised end to end. The tag lives in `<head>` so the per-snippet body wipe leaves it connected and so DOM cases counting body children never see the tag itself. Before each snippet the driver wipes `document.body`, `localStorage`, `sessionStorage`, and any app-created IndexedDB databases (the runtime's own cache is left intact).

```bash
deno test --allow-all tests/
```

`HOSTCAP=<cap>` narrows discovery to a single capability; CI uses it to fan out the matrix.

## Capability kinds

The synthesized manifest follows the capability's structure: a `src/entry.py` is imported as a code module (`imports`); otherwise the `src/index.js` host module is imported (`host`). The `.py` takes precedence when both exist.

| Has | Imported entry |
|---|---|
| `src/entry.py` | the `.py` (code module) |
| `src/index.js` (no `.py`) | the JS host module |

## Corpus shape

Each `<cap>/<cap>.json` is an array of cases. Per case:

| Field | Type | Purpose |
|---|---|---|
| `src` | string | Edge Python source. Driver prepends `from <cap> import *\n`. |
| `output` | string[] | Expected stdout lines. |
| `error` | string | Expected substring of the trace (use instead of `output`). |
| `html` | string | Optional `document.body.innerHTML` fixture. |
| `http_mocks` | `{url, body, status?, contentType?}[]` | `page.route` patterns fulfilled per call. |
| `ws_mocks` | `{url, echo?}[]` | `page.routeWebSocket` patterns; `echo: true` reflects messages back. |
