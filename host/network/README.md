# Edge Python Network

HTTP, WebSocket, and SSE shipped as a plain ESM module. Scripts see `network` as ordinary. Register it with `createWorker({ mainThreadModules: { network } })` (or the `host` field of `packages.json`); see [`host/README.md`](../README.md) for the setup boilerplate.

```python
from network import fetch_json, ws_open, ws_send
from "https://cdn.edgepython.com/std/json.wasm" import loads

data = fetch_json("https://api.example.com/users") # yields, composes with gather / with_timeout

sock = ws_open("wss://example.com/socket", "msg") # streaming, push-event pattern
ws_send(sock, "hello")
async def main():
    while True:
        ev = loads(receive())
        if ev["type"] == "message":
            print(ev["data"])
```

## Testing

```bash
deno run -A npm:playwright install chromium # one-time
HOSTCAP=network deno test --allow-all tests/ # from repo root
```

See [`tests/README.md`](../tests/README.md) for the corpus shape.

## API

### Conventions

- **HTTP handlers yield.** They return a Promise on the JS side; the runtime parks the coro in `WaitingHostCall` until it resolves, Python sees a sync-looking call that suspends. `gather()`, `with_timeout()`, and `run()` work over them with no `await`/`receive()`.
- **WebSocket/SSE use push-events** (like `dom`'s `bind_event`). Connections open with a `msg` tag; every wire event arrives via `receive()` as JSON.
- **Handles are integer IDs.**
- **Options are JSON strings**, `fetch(url, '{"method":"POST","body":"..."}')`.
- **Response bodies are strings.** Parse JSON with the [`json`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/json) standard package (not built-in: declare it via a `packages.json` alias or import it by URL).

### HTTP

`fetch`, `fetch_text`, `fetch_json`, `abort_request`.

```python
# Full response object as a JSON string: {id, ok, status, headers, body}
resp = loads(fetch("https://api.example.com/users"))

# Convenience helpers: body string only. Raise on non-2xx.
text = fetch_text("https://example.com")
data = loads(fetch_json("https://api.example.com/users"))

# POST with options; abort an in-flight request by its `id`
resp = loads(fetch("https://api.example.com/users",
    '{"method":"POST","body":"{\\"name\\":\\"ada\\"}","headers":{"Content-Type":"application/json"}}'))
abort_request(resp["id"])
```

### Concurrency (free from the scheduler)

`gather()` and `with_timeout()` work directly over the yielding HTTP handlers:

```python
results = gather( # three requests in parallel
    fetch("https://api.example.com/a"),
    fetch("https://api.example.com/b"),
)

try:
    body = with_timeout(2.0, fetch_text("https://slow.example.com"))
except TimeoutError:
    print("too slow")
```

An `async def` call returns a coroutine without running it, so a comprehension builds the batch and `gather(*...)` runs it. Each coroutine parks at its own `fetch` (a deferred host call tagged with a unique id); the host resolves the in-flight requests concurrently and delivers every response back to the coroutine that issued it. A rejected request raises in that one coroutine, so `try/except` isolates it from the batch.

```python
async def load(url):
    try:
        return fetch_text(url)
    except:
        return None # failed requests don't abort the batch

urls = [f"https://api.example.com/item/{i}" for i in range(1000)]
bodies = gather(*[load(u) for u in urls])
```

This is concurrency, not parallelism: the VM runs on one thread, so requests overlap while in flight but coroutines resume one at a time. Throughput is bounded by the browser's per-host connection limit (~6 on HTTP/1.1, multiplexed on HTTP/2), memory, and bandwidth, not by the scheduler.

### WebSocket

`ws_open(url, msg, protocols_json?)`, `ws_send(handle, data)`, `ws_close(handle, code?, reason?)`, `ws_state(handle)`.

```python
sock = ws_open("wss://example.com/socket", "ws")

async def main():
    while True:
        ev = loads(receive())
        if ev["type"] == "open":
            ws_send(sock, "hello")
        elif ev["type"] == "message":
            print(ev["data"])
        elif ev["type"] == "close":
            return

run(main())
```

Payload `type` values: `open`, `message`, `close`, `error`. `message` carries `data` for text frames (binary frames surface `binary: true` only; bidirectional bytes is a future addition).

### Server-Sent Events

`sse_open(url, msg, options_json?)`, `sse_close(handle)`, `sse_state(handle)`.

```python
stream = sse_open("/events", "sse")

async def main():
    while True:
        ev = loads(receive())
        if ev["type"] == "message":
            print(ev["data"])
        elif ev["type"] == "error":
            sse_close(stream)
            return

run(main())
```

## How it works

`src/index.js` is a factory `(ctx) => handlers` (same shape as `dom`). Three slices in `src/` (`http`, `ws`, `sse`) close over a shared `state` (handle tables for in-flight requests, sockets, SSE sources) and merge with `Object.assign`. HTTP handlers are async (`async (url) => { ... return body; }`); the runtime detects the Promise and parks in `WaitingHostCall` until resolved, same shape as `sleep()`. WS/SSE slices return sync handlers wiring DOM-style listeners into `ctx.pushEvent`.

Per-handler cost is one `postMessage` round-trip; HTTP adds network latency on top. For many small same-host requests, prefer one larger request. JS sources only; loads from `cdn.edgepython.com`, no build step.

## License

Apache-2.0
