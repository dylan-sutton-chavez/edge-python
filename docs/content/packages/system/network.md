---
title: "network (js, cli)"
description: "HTTP fetch, WebSocket, and Server-Sent Events."
---

`network` is HTTP, WebSocket, and Server-Sent Events from scripts. Declare it with `edge add network` and import it by bare name. It runs in the JS host, in a browser or elsewhere, and in the CLI, which builds it in and maps the entry's CDN URL to it, see [The CLI](/reference/modules#the-cli).

The surface is `fetch`, `fetch_text`, `fetch_json`, `abort_request`, plus WebSocket (`ws_open`, `ws_send`, `ws_close`, `ws_state`) and Server-Sent Events (`sse_open`, `sse_close`, `sse_state`). HTTP calls suspend until the response arrives. `fetch` returns the full response as a JSON string with `id`, `ok`, `status`, `headers`, and `body`, and `abort_request(id)` cancels an in-flight request. `fetch_text` returns the body as a string and `fetch_json` does the same for you to parse with `json.loads`. Both raise on a non-2xx status. All three take an optional second argument, a JSON options string (`RequestInit` in the JS host). WebSocket and SSE connections open with a `msg` tag and stream events through `receive()`, with payload `type` values `open`, `message`, `close`, and `error`. Binary WebSocket frames surface as `binary: true` only. In the browser, CORS applies: a cross-origin target must return `Access-Control-Allow-Origin` or the call raises. CORS is a browser rule and does not apply in the CLI. The CLI has every name except `abort_request`, importing it there fails at compile time.

```python
from network import fetch, fetch_text
import json

data = json.loads(fetch("https://api.github.com/zen"))
print(data["ok"], data["status"])
print(len(fetch_text("https://api.github.com/zen")) > 0)
```

```text Output
True 200
True
```

```python
from network import fetch_json, fetch_text
import json

data = json.loads(fetch_json("https://api.github.com/"))
print(isinstance(data, dict))

try:
  fetch_text("https://nope.invalid/x")
except Exception as e:
  print(type(e).__name__)
```

```text Output
True
RuntimeError
```

Known limitations: the CLI suspends on `fetch` and supports WebSocket and SSE like the JS host. The irreducible difference is CORS, a browser rule that does not apply in the CLI, and per-host connection limits that differ between the two.
