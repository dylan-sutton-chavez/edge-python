---
title: "Actors"
description: "Run many isolated edge-python programs as cooperative tasks over a few threads."
---

An actor pool runs many edge-python programs as cooperative tasks multiplexed over a few threads, not one OS thread per program. Each actor is its own interpreter with its own heap, they share nothing and talk only by message. It serves two shapes of work. You orchestrate your own programs as a pipeline of cooperating groups, or you run untrusted code from clients, each in its own sandbox. Both run from an `actor.yml`.

```bash
edge actor actor.yml
```

The pool boots the groups, runs to quiescence, and exits, or stays up as a server when a `listen:` address is set.

## Groups

A group is one program run as a pool of interchangeable actors. It gets its program one of three ways.

```yaml
groups:
  actor:
    code: |            # an inline program
      msg = receive()
      print(msg)
  parser:
    run: app           # a main.py or a whole project directory
  runner:
    eval: true         # compile each message as its own program
```

`code:` is an inline body. `run:` points at a script or a project directory, and a directory runs its `main.py` and resolves that project's `edge.json` and nested imports. `eval: true` is untrusted mode, covered below.

## Actors and load

`replicas:` is a ceiling, not a count. Actors are born on demand up to it and an idle actor costs about 31 KB, so a group declares a large ceiling and pays only for the actors actually running. A message is handed to an idle actor first, then to a fresh one under the ceiling, then to the least-loaded live actor once the group is saturated.

```yaml
groups:
  actor:
    run: app
    replicas: 100000   # ceiling, actors spawn as work arrives
```

## Messages

An actor loops over `receive()` and sends with the `actor` module. Sends are fire-and-forward, an actor never blocks waiting on another, which keeps a pool free of circular deadlock.

```python
from actor import send

msg = receive()
send("transform", msg + "-done")   # hand it to the transform group
```

Like every module, `actor` must be declared. Groups resolve their imports through the `edge.json` beside `actor.yml`, or the one `--manifest` names, so `edge add actor` there is the first step of any pool that sends. A group's `seed:` list delivers messages before the pool starts, the entry point that kicks a run off.

## Group fields

| Field | Meaning |
|-------|---------|
| `code` | Inline program body |
| `run` | Script path or project directory to run |
| `eval` | Untrusted mode, compile each message as its own program |
| `replicas` | Actor ceiling, actors spawn on demand up to it |
| `retry` | Times a crashing message is retried before it is dropped |
| `seed` | Messages delivered before the pool starts |
| `out` | Where `print` goes, `stdout`, `null`, or `file://path` |
| `limits` | Per-actor `heap`, `ops`, `calls`, and `preempt` overrides |

## The server

A `listen:` address turns the pool into a live server. It stays up instead of ending at quiescence, and its ingress accepts messages over TCP.

```yaml
runtime:
  listen: tcp://127.0.0.1:7777
  durable: tmp/actor/log
  control: tcp://127.0.0.1:9090
  schedulers: auto     # one per core, or a fixed number
  max_actors: 1000000   # ceiling across every group
```

A client connects and sends one `<group> <body>` line per message over tcp, or posts the body to `/pub/<group>` on the control address when http fits better. Either way the body reaches an actor of that group through `receive()`.

```bash
$ curl -X POST localhost:9090/pub/actor -d 'hello'
{"ok":true}   # 202, fire and forget like the tcp line
```

A `durable:` path logs every message and replays what was unprocessed on restart, so a crash loses nothing. A `control:` address serves live counts at `/stats`, and the response itself proves the pool is alive. It also takes published messages at `/pub/<group>` and answers eval runs at `/eval/<group>`, covered in untrusted code below.

```bash
$ curl localhost:9090/stats
{"actors":4,"active":1,"idle":3,"pending":0,"crashes":0,"dead":0}
```

| Field | Meaning |
|-------|---------|
| `actors` | Live actors across every group |
| `active` | Actors running a message right now |
| `idle` | Actors parked on `receive()` with an empty mailbox |
| `pending` | Messages queued and not yet delivered |
| `crashes` | Actors retired after an uncaught error |
| `dead` | Messages dropped after exhausting their retries |

## Failure

An actor that raises is retired with its traceback. `retry:` re-delivers the message it was processing to another actor up to that many times, then drops it to the dead count so one poison message cannot take a group down. A group's actors share a few wasm instances, so a fault in the engine itself retires every actor of the instance it hit the same way, and their queued messages move on to other actors.

```yaml
groups:
  actor:
    run: app
    retry: 2           # three attempts, then the message is dropped dead
```

## Untrusted code

An `eval` group runs code it does not trust. Each incoming message is compiled and run in a fresh wasm instance with its own linear memory, capped by the group's `heap` limit inside the interpreter and by a 256 MiB reservation outside it, and cut off after ten seconds of wall-clock time by a deadline the host enforces from outside the instance, so a runaway loop or a long native operation ends even where the interpreter cannot yield. Nothing survives between messages, the instance is dropped when the run ends. A bundle that carries its own `edge.json` resolves through it, any other message through the pool's manifest, and either way `actor`, `network`, and `.wasm` plugins are refused, so untrusted code cannot send to other groups, reach the network, or load modules from disk.

A `code` or `run` group runs code you trust. It keeps state between messages, can send, and can reach `network`, with the metered limits as the only cap, so keep third-party code out of it and reach for `eval` instead.

For a project, `edge build --bundle` packs it into a `.package`, and a client sends it base64-encoded behind an `EDGEPKG:` marker. The actor validates it and serves its files to the compiler from memory, so an untrusted bundle never touches the disk.

```yaml
groups:
  runners:
    eval: true         # every message is untrusted, no send, no shared state
```

A client that wants the result posts the snippet or bundle to `/eval/<group>` on the control address, and the reply carries what the run printed.

```bash
$ curl -X POST localhost:9090/eval/runners -d 'print(2 + 3)'
{"ok":true,"stdout":"5\n"}
```

A run that raises answers `{"ok":false,"error":...}` with its traceback, and the caller waits thirty seconds at most before a 504. The TCP ingress stays fire and forget, only these posts get a reply.

## A three-stage pipeline

A seed flows through three groups, each stage sending to the next. The `edge.json` beside the manifest declares `actor` for the two stages that send.

```json
{ "imports": { "actor": "https://cdn.edgepython.com/js/builtins/actor/index.js" } }
```

```yaml
groups:
  ingest:
    seed: ["raw"]
    code: |
      from actor import send
      send("transform", receive() + "-ingested")
  transform:
    code: |
      from actor import send
      send("sink", receive() + "-transformed")
  sink:
    code: |
      print("sink:", receive())
```

```text
$ edge actor actor.yml
sink: raw-ingested-transformed
```

## See also

- [CLI](/reference/cli) for `edge actor` and `edge build --bundle`.
- [Async](/language/async) for the cooperative scheduler each actor runs on.
- [Snapshots](/language/snapshots) for freezing and resuming a single run.
