---
title: "dom (web)"
description: "The browser DOM, from element queries to observers and animations."
---

`dom` wraps the browser DOM. It is a JavaScript module on the browser's main thread, where `document` and `window` live. Import it by bare name or declare it in the `system` field of `packages.json`. The native engine rejects `import dom` at compile time, see [the native engine](/reference/modules#the-native-engine).

The engine runs in a Web Worker, so each call crosses over `postMessage` and the script sees a synchronous call. Handlers that return a Promise suspend the calling coroutine and compose with `gather`, see [Async](/language/async).

Importing `dom` gives you a script-level facade over the internal `_dom` module that adds opt-in mutation batching. Inside a `batch()` block, mutators buffer locally and the block exit applies them with one host call. `pending()` counts buffered ops and `flush()` applies them early. Handles are opaque integers. Multi-result queries return CSV strings of handles. Structured returns are JSON strings, parse them with [json](/packages/std/json). Async results (events, file reads, observer entries, animation finishes) arrive through `receive()` carrying a correlation handle. Detaching a node sweeps its handles, event bindings, and animations. `bind_global_error(msg)` routes async DOM callback errors to `receive()` instead of the browser console. The full handler inventory, by group:

- Selection and traversal: `query`, `query_all`, `closest`, `matches`, `body`, `active_element`, `parent`, `children`, `first_child`, `last_child`, `next_sibling`, `prev_sibling`, `tag_name`.
- Creation and mutation: `create_element`, `create_element_ns`, `append_child`, `insert_before`, `remove`, `replace_children`, `clone_node`.
- Content, attributes, classes: `get_text`, `set_text`, `get_html`, `set_html`, `get_attribute`, `set_attribute`, `remove_attribute`, `add_class`, `remove_class`, `toggle_class`, `has_class`, `get_data`, `set_data`.
- Style and layout: `set_style`, `get_style`, `get_computed_style`, `rect`, `offset_width`, `offset_height`, `client_width`, `client_height`, `scroll_top`, `set_scroll_top`, `scroll_into_view`, `focus`, `blur`.
- Events: `bind_event`, `unbind_event`, `dispatch_event`, `click`. Each fire delivers a JSON payload with `msg`, `target_handle`, and the event fields.
- Forms: `get_value`, `set_value`, `get_checked`, `set_checked`, `form_submit`, `form_reset`, `form_data`, `is_valid`, `validity`, `report_validity`, `set_custom_validity`, `validation_message`.
- Files: `get_files`, `file_info`, `file_read_text`, `file_read_data_url`.
- Observers: `observe_intersection`, `observe_resize`, `observe_mutations`, each with an `unobserve_*` counterpart.
- Animations: `animate`, `animation_play`, `animation_pause`, `animation_cancel`, `animation_finish`, `animation_reverse`, `animation_dispose`.
- Media: `media_play`, `media_pause`, `get_current_time`, `set_current_time`, `get_duration`, `get_paused`, `set_volume`, `set_playback_rate`.
- Platform: `show_modal`, `dialog_close`, `request_fullscreen`, `exit_fullscreen`, `fullscreen_element`, `request_pointer_lock`, `exit_pointer_lock`, plus SVG introspection `bbox`, `path_length`, `point_at_length`.
