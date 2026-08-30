---
title: "Quickstart"
description: "Install the CLI, run a script, import a package."
---

## Install the CLI

```bash
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh
```

The installer works on macOS, Linux, and WSL. Open a new shell and check the install:

```bash
edge --version
```

To build from source instead, run `cargo install --path cli` from the repo root. See the [CLI reference](/reference/cli) for install details and flags.

## Run your first script

Create `hello.py`:

```python
def greet(name):
  return f"Hello, {name}!"

for who in ["world", "edge", "python"]:
  print(greet(who))
```

```text Output
Hello, world!
Hello, edge!
Hello, python!
```

Run it from your terminal:

```bash
edge run hello.py
```

There is no build step. The CLI compiles the file and runs it in its native engine.

## Import your first package

Edge Python ships no standard library. The official packages resolve by bare name, with no configuration. Create `app.py`:

```python
import json

data = json.loads('{"name": "edge", "tags": ["fast", "small"]}')
data["tags"].append("sandboxed")
print(json.dumps(data))
```

```text Output
{"name":"edge","tags":["fast","small","sandboxed"]}
```

Run it:

```bash
edge run app.py
```

To record the dependency in your project, run `edge add json`. It writes the package to `packages.json`. The package catalog lives in [Modules](/reference/modules#standard-packages) and the manifest format in [packages.json](/reference/modules#packagesjson).

## Next steps

- [Syntax](/language/syntax) walks through the language.
- [Command line interface](/reference/cli) covers `serve`, `repl`, `test`, `build`, and `actor`.
- [Actors](/reference/actors) runs many programs as cooperative tasks.
