---
title: "actor (cli)"
description: "Message passing between the actors of a actor pool."
---

`actor` is message passing inside an [actor pool](/reference/actors). Declare it with `edge add actor`, a `system` entry, and import it by bare name. The CLI builds it in and resolves the entry by name, and it is meaningful only to programs running under `edge actor`. The JS host loads the CDN stub the entry points at, and its `send` throws `actor.send needs the CLI` at the first call.

The surface is `send`. `send(group, body)` queues a message to a group, fire and forget, and a actor of that group picks it up with the `receive()` builtin, which needs no import.

```python
from actor import send

msg = receive()
send("transform", msg + "-done")
```

A send never blocks, so a pool stays free of circular deadlock. Eval groups run untrusted code without it, importing `actor` from a snippet fails the run.
