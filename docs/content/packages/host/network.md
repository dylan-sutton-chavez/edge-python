---
title: "network (web, native)"
description: "HTTP fetch, WebSocket, and Server-Sent Events."
---

`network` is HTTP, WebSocket, and Server-Sent Events from scripts. Import it by bare name or declare it in the `host` field of `packages.json`. The native engine builds it in, see [the native engine](/reference/modules#the-native-engine).

The surface is `fetch`, `fetch_text`, `fetch_json`, `abort_request`, plus WebSocket (`ws_open`, `ws_send`, `ws_close`, `ws_state`) and Server-Sent Events (`sse_open`, `sse_close`, `sse_state`). HTTP calls suspend until the response arrives. `fetch` returns the full response as a JSON string with `id`, `ok`, `status`, `headers`, and `body`, and `abort_request(id)` cancels an in-flight request. `fetch_text` returns the body as a string and `fetch_json` does the same for you to parse with `json.loads`. Both raise on a non-2xx status. All three take an optional second argument, a JSON options string (`RequestInit` in the browser). WebSocket and SSE connections open with a `msg` tag and stream events through `receive()`, with payload `type` values `open`, `message`, `close`, and `error`. Binary WebSocket frames surface as `binary: true` only. In the browser, CORS applies: a cross-origin target must return `Access-Control-Allow-Origin` or the call raises. CORS is a browser rule and does not apply on other hosts.

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

Known limitations: the native engine suspends on `fetch` and supports WebSocket and SSE like the browser. The irreducible difference is CORS, a browser rule that does not apply natively, and per-host connection limits that differ between the two.
