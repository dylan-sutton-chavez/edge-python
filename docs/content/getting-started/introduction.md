---
title: "Introduction"
description: "What Edge Python is, what it is not, and where to go next."
---

Edge Python is a sandboxed subset of Python. The compiler and the virtual machine are written in Rust. You write ordinary Python syntax. The compiler turns it into bytecode, and a stack VM runs it.

The engine is one WebAssembly binary of about 200 KB, and two hosts run it:

- The JS host, the JavaScript package. It is built on the browser's sandbox model over Web APIs, so it runs in browsers and in JavaScript runtimes that offer the same APIs, such as Deno. `dom`, `storage`, `frame()` and the `<edge-python>` element need a browser.
- The `edge` command line interface, a Rust binary that embeds the same engine, built from the same source for speed, precompiled and run under wasmtime.

Sandboxed means the defaults deny everything. A program gets no file system, no network, and no environment access unless the host grants it. Imports resolve at compile time through a resolver the host injects, so a running program never loads code you did not declare, the official packages included. Every module a project uses is an entry in its `edge.json`.

Every runnable example in these docs executes on the real engine through the JS host. Try editing this one:

```python
text = "the quick brown fox"
words = {w: len(w) for w in text.split()}

for w, n in words.items():
  print(f"{w:>6} -> {n}")
```

```text Output
   the -> 3
 quick -> 5
 brown -> 5
   fox -> 3
```

## What it is not

Edge Python is not a full Python. It leaves out what does not fit a small sandboxed engine:

- No bundled standard library. Every module is an external package. See [Modules](/reference/modules).
- No dynamic code. `exec`, `eval`, `compile`, and `__import__` do not exist.
- No metaclasses, no descriptor protocol, no `__slots__`.
- No `complex` numbers.
- A fixed set of builtins and nothing more. `dir` is not one of them. See [Builtins](/reference/builtins).

The [language guide](/language/syntax) covers everything that is supported.

## Where to go next

- [Quickstart](/getting-started/quickstart) installs the CLI and runs your first program.
- [Syntax](/language/syntax) walks through the language feature by feature.
- [Builtins](/reference/builtins) and [Methods](/reference/methods) list the fixed builtin set.
- [Design](/implementation/design) explains how the compiler and VM work.

Machine learning models can load `skill/SKILL.md` from the repository for a self-verifying summary of the language and CLI.

For questions or integration help, email [dylan@edgepython.com](mailto:dylan@edgepython.com).
