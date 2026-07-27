---
title: "Quickstart"
description: "Run your first Edge Python program in under a minute."
---

## Run it

Edge Python ships as a ~200 KB WASM module. The fastest way to try it is any runnable code block in these docs ([here](/getting-started/introduction#try-it), for instance). No install. Everything runs client-side.

For a local project, the [CLI](/reference/cli) covers the whole loop:

```bash
edge init my-app # scaffold entry, packages.json, and a test
cd my-app && edge serve # dev server with live reload
edge test # run the *_test.py files
```

## Embed it

To put Edge Python in your own page, drop in the `<edge-python>` element below. Building the `.wasm` from source or embedding in Rust/WASI: see [Where it runs](/getting-started/what-it-is#where-it-runs).

### Drop-in HTML element

In the browser, the `<edge-python>` element runs a `.py` file declaratively. No JS wiring. With a host library like `dom` (declared in `packages.json`), the script renders straight into the page:

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <script type="module" src="https://cdn.edgepython.com/runtime/src/element.js"></script>
</head>
<body>
  <div id="app"></div>
  <edge-python entry="./app/hello.py" packages="./app/packages.json"></edge-python>
</body>
</html>
```

```json
// packages.json
{ "host": { "dom": "./dom/src/index.js" } }
```

```python
# hello.py
from dom import query, set_text
set_text(query("#app"), "Hello from Python")
```

`dom` is one of the official [host libraries](/reference/packages#host-libraries) (`dom`, `network`, `storage`...). Standard `.wasm` packages like [`json`](/reference/packages#json) sit alongside them. Official packages also resolve by bare name with no manifest, fetched lazily on first import (see [Defaults](/reference/packages#defaults)).

See [Official packages](/reference/packages) for the full catalog. See the [runtime README](https://github.com/dylan-sutton-chavez/edge-python/tree/main/js) for all `<edge-python>` attributes and the `imports` field for `.py` / `.wasm` modules.

## Your first program

Try to run or modify this script:

```python
greet = lambda name: f"Hello, {name}!"

for who in ["world", "edge", "python"]:
  print(greet(who))
```

```text Output
Hello, world!
Hello, edge!
Hello, python!
```
