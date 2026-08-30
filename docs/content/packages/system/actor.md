---
title: "actor (native)"
description: "Message passing between the actors of a actor pool."
---

`actor` is message passing inside a [actor pool](/reference/actors). Import it by bare name, the native engine builds it in, and it is meaningful to programs running under `edge actor`.

The surface is `send`. `send(group, body)` queues a message to a group, fire and forget, and a actor of that group picks it up with the `receive()` builtin, which needs no import.

```python
from actor import send

msg = receive()
send("transform", msg + "-done")
```

A send never blocks, so a pool stays free of circular deadlock. Eval groups run untrusted code without it, importing `actor` from a snippet fails the run.
