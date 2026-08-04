---
title: "Introduction"
description: "What Edge Python is and where to go next."
---

Welcome to the Edge Python docs! 👋 Edge is a sandboxed subset of Python, built in Rust to run in the browser as a ~200 KB WebAssembly binary and natively inside the CLI. Embed your full business logic, run LLMs client-side, and build frontend apps.

## Ecosystem

1. [Quickstart](/getting-started/quickstart): Run your first Edge Python program in under a minute.
2. [What Edge Python is](/getting-started/what-it-is): Scope, philosophy, and what intentionally isn't supported.
3. [Syntax](/language/syntax): The language guide.
4. [Reference](/reference/builtins): All the builtin functions and methods.
5. [Implementation](/implementation/design): Compiler architecture, dispatch model, and runtime layout.

## Try it

### Browser

Try to edit or execute this script:

```python
import time

text = "the quick brown fox"
words = {w: len(w) for w in text.split()}

for w, n in words.items():
  print(f"{w:>6} -> {n}")
  time.sleep(0.2)
```

```text Output
   the -> 3
 quick -> 5
 brown -> 5
   fox -> 3
```

### Command Line Interface

Or download it to your machine ([reference docs](/reference/cli)):

```bash
# Compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

edge -h # List all commands
```

## Need help?

Looking to integrate Edge into your app: run Python business logic in your users' browsers, or anything else? Get in touch: [email](mailto:c.sutton.dylan@gmail.com)
