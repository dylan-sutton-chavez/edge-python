---
title: "swarm (native)"
description: "Message passing between the nodes of a worker pool."
---

`swarm` is message passing inside a [worker pool](/reference/workers). Import it by bare name, the native engine builds it in, and it is meaningful to programs running under `edge swarm`.

The surface is `send`. `send(group, body)` queues a message to a group, fire and forget, and a node of that group picks it up with the `receive()` builtin, which needs no import.

```python
from swarm import send

msg = receive()
send("transform", msg + "-done")
```

A send never blocks, so a pool stays free of circular deadlock. Eval groups run untrusted code without it, importing `swarm` from a snippet fails the run.
