# Edge Python DOM

DOM access shipped as a plain ESM module. Scripts see `dom` as ordinary, full surface: queries, mutation, events, forms, files, observers, animations, layout, media, SVG, dialog, fullscreen, pointer lock.

```python
from dom import query, set_text, bind_event

async def main():
    bind_event(query("#btn"), "click", "click")
    n = 0
    while True:
        receive() # payload string; ignore content for a simple counter
        n += 1
        set_text(query("#btn"), f"clicked {n} times")
```

Two pieces, one import name: the JS handlers register as the internal `_dom` module (`createWorker({ mainThreadModules: { _dom: dom } })`, or the `host` field of `packages.json`), and Python imports the [`src/entry.py`](src/entry.py) façade declared as `dom` under `imports` — it re-exports every handler and adds [batching](#batched-mutations). 

See [`host/README.md`](../README.md) for the setup boilerplate and [`packages.json`](packages.json) for the manifest pair. Engine runs in a Web Worker; `dom` handlers run on the page's main thread (where `document` lives) via the runtime's deferred host-call mechanism. Python sees every call as synchronous.

## Testing

Cases live in [`dom.json`](dom.json) and run through the shared runner at the repo root:

```bash
deno run -A npm:playwright install chromium # one-time
HOSTCAP=dom deno test --allow-all tests/ # from repo root
```

See [`tests/README.md`](../tests/README.md) for the corpus shape.

## API

**Conventions:**

- Handles are opaque integers; store, pass, never compute on them.
- Multi-result queries (`query_all`, `children`) return CSV strings of handles.
- Structured returns (`rect`, `validity`, `form_data`, `bbox`, event payloads) are JSON strings.
- Async results (events, FileReader, animation finishes, observer entries) arrive via `receive()`.
- Async payloads carry a correlation handle so the consumer can route results back to the originating call: `target_handle` (events, observers), `file_handle` (file_read_*), `animation_handle` (animate finish).
- Cleanup is automatic. Detaching nodes (`remove`, `replace_children`, `set_html`, `set_text`) sweeps the subtree's handles, event bindings, and active animations. `file_read_*` releases its file handle on completion. Animations auto-release on finish or cancel, except `iterations: "Infinity"` loops, which need `animation_dispose(h)`.

**Parsing JSON returns.** Several handlers return JSON strings; parse them with the [`json`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/json) standard package (not built-in: declare it via a `packages.json` alias or import it by URL).

### Selection and traversal

`query`, `query_all`, `closest`, `matches`, `body`, `active_element`, `parent`, `children`, `first_child`, `last_child`, `next_sibling`, `prev_sibling`, `tag_name`.

```python
card = query("#card")
items = [int(h) for h in query_all("li.todo").split(",") if h]
```

### Creation and mutation

`create_element`, `create_element_ns`, `append_child`, `insert_before`, `remove`, `replace_children`, `clone_node`.

```python
li = create_element("li")
set_text(li, "row")
append_child(query("#list"), li)
replace_children(query("#list")) # atomic clear
```

### Content, attributes, classes

`get_text`, `set_text`, `get_html`, `set_html`, `get_attribute`, `set_attribute`, `remove_attribute`, `add_class`, `remove_class`, `toggle_class`, `has_class`, `get_data`, `set_data`.

```python
set_html(query("#body"), "<p>Inline markup</p>")
toggle_class(query("#menu"), "open")
set_data(query("#row"), "id", "42") # element.dataset.id = "42"
```

### Style and layout

`set_style`, `get_style`, `get_computed_style`, `rect`, `offset_width`, `offset_height`, `client_width`, `client_height`, `scroll_top`, `set_scroll_top`, `scroll_into_view`, `focus`, `blur`.

```python
set_style(query("#box"), "transform", "translateX(20px)")
r = loads(rect(query("#box"))) # {"w","h","x","y"}
```

### Events

`bind_event(node, type, msg, options_json?)`, `unbind_event(h)`, `dispatch_event(node, type, detail?)`, `click(node)`.

```python
bind_event(query("#form"), "submit", "submit", '{"prevent_default": true}')

async def main():
    while True:
        ev = loads(receive())
        if ev["msg"] == "submit":
            print("submitted")
```

Each fire dispatches a JSON detail. Payload fields: `msg`, `target_handle`, `type`, `target_id`, `target_tag`, `value`, `checked`, `key`, `code`, `button`, `x`, `y`, `movement_x`/`y`, `alt`/`ctrl`/`shift`/`meta`, plus `drop_files`, `drop_text`, `clipboard_text`, `clipboard_files`, `touches` when applicable. `target_handle` is the bound element (useful when sharing a `msg`). Options JSON: `prevent_default`, `stop_propagation`, `once`, `capture`, `passive`.

### Forms

`get_value`, `set_value`, `get_checked`, `set_checked`, `form_submit`, `form_reset`, `form_data`, `is_valid`, `validity`, `report_validity`, `set_custom_validity`, `validation_message`.

```python
v = loads(validity(query("#email")))
if v["type_mismatch"]:
    set_custom_validity(query("#email"), "Please use a valid email")
data = loads(form_data(query("#signup"))) # {"email": ["x@y.com"], "remember": ["on"]}
```

### Files

`get_files`, `file_info`, `file_read_text`, `file_read_data_url`.

```python
bind_event(query("#picker"), "change", "picked")

async def main():
    while True:
        ev = loads(receive())
        if ev["msg"] == "picked":
            for h in [int(x) for x in get_files(query("#picker")).split(",") if x]:
                file_read_data_url(h, "loaded")
        elif ev["msg"] == "loaded" and ev["ok"]:
            set_attribute(query("#preview"), "src", ev["data_url"])
```

Reads are async; result via `receive()` as `{msg, file_handle, ok, text | data_url}` (or `{..., ok: false, error}`). The file handle releases on completion (one read per handle; re-pick or re-drop to read again). Dropped and pasted files also surface as handles in `bind_event` payloads (`drop_files`, `clipboard_files`).

### Observers

`observe_intersection`, `observe_resize`, `observe_mutations` and their `unobserve_*` counterparts.

```python
observe_intersection(query("#hero"), "visible", '{"threshold": 0.5}')

async def main():
    while True:
        ev = loads(receive())
        if ev["msg"] == "visible" and ev["intersecting"]:
            set_attribute(query("#hero img"), "src", "/large.jpg")
```

Each fires a payload through `receive()` per entry with observer-specific fields. All payloads carry `target_handle` (the handle passed to `observe_*`) so a shared `msg` routes back to its source observer.

### Animations

`animate(node, keyframes_json, options_json, msg?)`, `animation_play`, `animation_pause`, `animation_cancel`, `animation_finish`, `animation_reverse`, `animation_dispose`.

```python
animate(query("#spinner"), '[{"transform": "rotate(0deg)"}, {"transform": "rotate(360deg)"}]', '{"duration": 1000, "iterations": "Infinity"}')
```

Works on HTML and SVG. Returns a handle. With a `msg`, a `{msg, animation_handle, ok}` payload fires via `receive()` on finish. Handle auto-disposes on finish or cancel; for `iterations: "Infinity"` call `animation_dispose(h)` when done.

### Media (`<video>` and `<audio>`)

`media_play`, `media_pause`, `get_current_time`, `set_current_time`, `get_duration`, `get_paused`, `set_volume`, `set_playback_rate`.

```python
vid = query("#movie")
if get_paused(vid):
    media_play(vid)
set_current_time(vid, 30.0) # seek
```

Bind to `timeupdate` / `ended` / `loadedmetadata` for state events.

### Platform (dialog, fullscreen, pointer lock, SVG)

`show_modal`, `dialog_close`, `request_fullscreen`, `exit_fullscreen`, `fullscreen_element`, `request_pointer_lock`, `exit_pointer_lock`, `bbox`, `path_length`, `point_at_length` (SVG geometric introspection).

```python
SVG = "http://www.w3.org/2000/svg"
circle = create_element_ns(SVG, "circle")
set_attribute(circle, "r", "40")
append_child(query("svg"), circle)
```

### Batched mutations

Inside a `batch()` block, mutators buffer locally instead of crossing per call; the block exit applies the whole buffer with a single host call. Outside a block every call is immediate, and reads (`get_text`, `query`, ...) are always immediate.

```python
from dom import query, batch, set_text, set_style

hud = query("#hud")
speed = query("#speed")

def frame(v):
    with batch():
        set_text(speed, str(v)) # buffered, no host call
        set_style(hud, "opacity", "1") # buffered, no host call
    # exit applies both with ONE host call
```

Batchable mutators: `set_text`, `set_html`, `set_attribute`, `remove_attribute`, `add_class`, `remove_class`, `set_data`, `append_child`, `insert_before`, `remove`, `replace_children`, `set_style`, `set_scroll_top`, `scroll_into_view`, `focus`, `blur`, `set_value`, `set_checked`. Nested `batch()` blocks coalesce into the outermost flush; an exception inside a block discards the buffer. `pending()` counts buffered ops and `flush()` applies them early. The raw op list rides `apply_batch(ops)` (`[name, ...args]` per op), which raises on any name outside the list above.

### Errors

Async errors in DOM callbacks (event listeners, observer callbacks, swallowed promise rejections in `media_play` / `request_fullscreen` / `request_pointer_lock` / `animate`) go to the browser console by default. Bind a message to surface them as ordinary Python events:

```python
from dom import bind_global_error

bind_global_error("err")

async def main():
    while True:
        ev = loads(receive())
        if ev["msg"] == "err":
            print(f"[{ev['where']}] {ev['error']}")
            continue
        # …rest of dispatch
```

Payload: `{msg, where, error, stack?}`. `where` identifies the call site (`event:click`, `media_play`, `observe_intersection`, …). With no binding set, errors fall back to `console.error` with no Python-visible signal.

## How it works

Factory `(ctx) => handlers`. `src/main/state.js` opens a fresh closure per `createWorker` with handle tables (`nodes`, `bindings`, `files`, observers, animations), `alloc` / `node` / `allocList` helpers, and `cleanSubtree`. Eight handler slices in `src/main/` (`tree`, `style`, `events`, `forms`, `observers`, `animations`, `media`, `platform`) each return an object literal of handlers closing over the shared state; `src/index.js` composes them with `Object.assign` and wraps async callbacks with `emitError` so failures surface via `bind_global_error`. 

Adding a handler is one entry in one slice; the [`entry.py`](src/entry.py) façade re-exports it automatically, and a new *mutator* also gets a buffered wrapper there plus an `index.js` `BATCHABLE` entry.

## Performance

Per-handler cost is one `postMessage` round-trip (~0.1–0.4 ms in modern browsers), plenty for UI-rate workloads at hundreds of ops/frame. Per-frame bursts of mutations should go inside a [`batch()`](#batched-mutations) block so the whole burst pays one round-trip. Bad fit: pixel-precise renders — pair with a `<canvas>` capability for the framebuffer path.

## Distribution

JS sources only; `compiler.wasm` and the runtime load from `cdn.edgepython.com` at page load. No vendored copy, no build step.

## License

MIT OR Apache-2.0
