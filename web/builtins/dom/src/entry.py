# dom façade: every _dom handler passes through; mutators buffer inside batch() and flush() applies them in one host call.
from _dom import *
from _dom import (
    apply_batch as _apply_batch,
    set_text as _set_text,
    set_html as _set_html,
    set_attribute as _set_attribute,
    remove_attribute as _remove_attribute,
    add_class as _add_class,
    remove_class as _remove_class,
    set_data as _set_data,
    append_child as _append_child,
    insert_before as _insert_before,
    remove as _remove,
    replace_children as _replace_children,
    set_style as _set_style,
    set_scroll_top as _set_scroll_top,
    scroll_into_view as _scroll_into_view,
    focus as _focus,
    blur as _blur,
    set_value as _set_value,
    set_checked as _set_checked,
)

# List containers: cross-function global rebinds don't propagate, mutation does.
_depth: list = [0]
_ops: list = []

class _Batch:
    def __enter__(self):
        _depth[0] += 1
        return self

    def __exit__(self, exc_type, exc, tb):
        _depth[0] -= 1
        if exc_type is not None:
            _ops.clear()
        elif _depth[0] == 0:
            flush()
        return False

def batch():
    # Buffer mutators until the block exits, then flush once. Nested blocks coalesce into the outermost flush; an exception discards the buffer.
    return _Batch()

def pending():
    # Number of buffered ops.
    return len(_ops)

def flush():
    # Apply buffered ops in order with a single host call.
    if not _ops:
        return
    _apply_batch(_ops)
    _ops.clear()

def set_text(h, text):
    if _depth[0]:
        _ops.append(["set_text", h, text])
    else:
        _set_text(h, text)

def set_html(h, html):
    if _depth[0]:
        _ops.append(["set_html", h, html])
    else:
        _set_html(h, html)

def set_attribute(h, name, value):
    if _depth[0]:
        _ops.append(["set_attribute", h, name, value])
    else:
        _set_attribute(h, name, value)

def remove_attribute(h, name):
    if _depth[0]:
        _ops.append(["remove_attribute", h, name])
    else:
        _remove_attribute(h, name)

def add_class(h, c):
    if _depth[0]:
        _ops.append(["add_class", h, c])
    else:
        _add_class(h, c)

def remove_class(h, c):
    if _depth[0]:
        _ops.append(["remove_class", h, c])
    else:
        _remove_class(h, c)

def set_data(h, key, value):
    if _depth[0]:
        _ops.append(["set_data", h, key, value])
    else:
        _set_data(h, key, value)

def append_child(parent, child):
    if _depth[0]:
        _ops.append(["append_child", parent, child])
    else:
        _append_child(parent, child)

def insert_before(new_h, ref_h):
    if _depth[0]:
        _ops.append(["insert_before", new_h, ref_h])
    else:
        _insert_before(new_h, ref_h)

def remove(h):
    if _depth[0]:
        _ops.append(["remove", h])
    else:
        _remove(h)

def replace_children(parent, *kids):
    if _depth[0]:
        _ops.append(["replace_children", parent] + list(kids))
    else:
        _replace_children(parent, *kids)

def set_style(h, prop, value):
    if _depth[0]:
        _ops.append(["set_style", h, prop, value])
    else:
        _set_style(h, prop, value)

def set_scroll_top(h, v):
    if _depth[0]:
        _ops.append(["set_scroll_top", h, v])
    else:
        _set_scroll_top(h, v)

def scroll_into_view(h):
    if _depth[0]:
        _ops.append(["scroll_into_view", h])
    else:
        _scroll_into_view(h)

def focus(h):
    if _depth[0]:
        _ops.append(["focus", h])
    else:
        _focus(h)

def blur(h):
    if _depth[0]:
        _ops.append(["blur", h])
    else:
        _blur(h)

def set_value(h, v):
    if _depth[0]:
        _ops.append(["set_value", h, v])
    else:
        _set_value(h, v)

def set_checked(h, v):
    if _depth[0]:
        _ops.append(["set_checked", h, v])
    else:
        _set_checked(h, v)
