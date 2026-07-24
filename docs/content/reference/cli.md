---
title: "Command Line Interface"
description: "The Edge Python developer command line interface (CLI): run, serve, repl, init, package management, and build."
---

The `edge` developer CLI. Write `.py`, run it, serve it, ship it. You never compile anything yourself. `edge` hosts the [Edge Python runtime](/getting-started/what-it-is#where-it-runs) in a headless Chromium provisioned at install time, then runs your code against it. You point it at a file.

```bash
edge run app.py     # run a script
edge serve          # dev server with live reload
edge repl           # interactive shell
edge test           # run *_test.py files (not implemented yet)
edge init my-app    # scaffold a project
edge add network    # add a package to packages.json
edge remove network # remove a package from packages.json
edge build          # bundle to dist/
edge uninstall      # remove the binary, PATH entry, optionally the bundled browser
```

The runtime does the actual work. `edge` is the loop around it. It launches the headless browser, serves the runtime alongside your code, runs everything in that browser, and streams output back to your terminal. `edge serve` opens the same setup in your own browser.

## Install

```bash
# Prebuilt binary (recommended), compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

# Or from source (any platform with Rust and Cargo)
cargo install --path cli
```

`install.sh` drops the binary at `~/.local/bin/edge` and appends that directory to your `~/.bashrc` or `~/.zshrc` if it isn't already on `PATH`. Open a new shell (or `source` the file it printed) and `edge --version` should work. Re-run the same `curl … | sh` line any time to upgrade. To remove everything: `curl -fsSL https://cdn.edgepython.com/cli/uninstall.sh | sh` (asks before removing the bundled browser cache).

`install.sh` also downloads a pinned `chrome-headless-shell` into `~/.cache/edge` when no browser is already reachable. This needs `unzip`, with no package manager and no `sudo`. An existing browser on `PATH`, or an `EDGE_CHROME_PATH` you set, is used as-is. Linux arm64 has no such build: install Chrome/Chromium manually and set `EDGE_CHROME_PATH`. See [Bring your own browser](#bring-your-own-browser).

## `edge run`: run a Python file

Runs a script and streams its output to the terminal. Imports resolve through [`packages.json`](/reference/imports#packagesjson). Uncaught errors print a traceback to stderr and exit with code 1.

```text
$ edge run hello.py
Hello from Edge Python
the sum is 42
```

```text
$ edge run broken.py
before
error: ZeroDivisionError: division by zero
  --> <input>:2:1
  |
2 | x = 1 / 0
  | ^
```

A `raise SystemExit(code)` with an integer (or no argument) exits cleanly with that code and no traceback. A string argument is reported as an error and exits 1.

Flags: `--packages <file>` (custom manifest). With no path, `edge run` reads from stdin if it is piped (`cat hello.py | edge run`). It errors out if stdin is a terminal.

## `edge serve`: local dev server

A dev server for browser apps. Serves your project directory and reloads the page on any file change via an injected polling client.

```text
$ edge serve
  http://localhost:5173
  watching .
```

Flags: `--port <n>` (default `5173`), `--open` (open the browser).

## `edge repl`: interactive shell

An interactive Edge Python shell for quick experiments.

```text
$ edge repl
Edge Python 0.1.0  ·  .exit, Ctrl+C or Ctrl+D to quit
>>> from math import sqrt, pi
>>> print(sqrt(2))
1.4142135623730951
>>> print([n * n for n in range(5)])
[0, 1, 4, 9, 16]
>>> .exit
```

History (arrow keys) and multi-line blocks (a line ending in `:` continues until a blank line) are supported. `.exit`, `Ctrl+C`, or `Ctrl+D` quit. `.reset` wipes the session state. Expression results are not auto-printed. Use `print()` explicitly.

The worker keeps one interpreter alive across prompts: each input compiles and runs **once**, and imports, definitions, and mutations persist in place. Side effects never re-fire, and an input that raises keeps the effects it made before the error.

## `edge test`: test runner

Not implemented yet. The `test` package itself (the harness you import) is available. `edge add test` writes it to `packages.json`, and both `edge run` and `edge serve` resolve it by default. A script can already `from test import fixture, test, raises, run` and call `run()` itself.

## `edge init`: scaffold a workspace

Scaffolds a ready-to-run project: an entry script, an HTML host page, and a manifest.

```text
$ edge init my-app
  created my-app/
    ├─ index.html
    ├─ main.py
    └─ packages.json

  next:
    cd my-app && edge serve
```

`--bare` skips `index.html` for script-only projects.

## `edge add` / `edge remove`: package manager

Manage [`packages.json`](/reference/imports#packagesjson) by name. `edge` knows the official std (`json`, `re`, `math`, `test`) and host (`dom`, `network`, `storage`, `time`) packages, so you don't paste URLs. Most std packages are `.wasm`. `test` is pure Edge Python, so it resolves to `test.py`. See [Official packages](/reference/packages) for the full catalog.

```text
$ edge add math network
  + math       std
  + network    host

  updated packages.json
```

```text
$ edge remove network
  - network

  updated packages.json
```

Point a package at a custom URL with `edge add foo=https://example.com/foo.wasm`.

## `edge build`: portable bundle

Bundles your app into a self-contained `dist/` for offline use or self-hosting. It vendors the runtime, the `compiler.wasm`, your scripts, and every package locally, so nothing is fetched at runtime.

```text
$ edge build
  (successful) vendored runtime
  (successful) fetched compiler.wasm
  (successful) vendored packages

  bundled to dist/

  13 runtime files + compiler.wasm
  2 packages
  3 scripts

  1.24 MB · 5.3s
```

Flags: `--out <dir>` (default `dist/`).

## `edge uninstall`

Removes the binary and its `PATH` entry. Asks before removing the bundled `chrome-headless-shell` cache (never touches system Chromium). Equivalent to the `uninstall.sh` one-liner in [Install](#install).

## Global flags

| Flag | Effect |
|------|--------|
| `--packages <file>` | Use a specific manifest instead of `./packages.json` |
| `--version` / `-V` | Print version |

`Ctrl+C` cancels any running command cleanly.

## Bring your own browser

`edge` uses, in order:

1. `EDGE_CHROME_PATH` if set.
2. A system `google-chrome` / `chromium` / `chrome` on `PATH` (or the `CHROME` env var).
3. The bundled `chrome-headless-shell` in `~/.cache/edge`.

`install.sh` downloads `chrome-headless-shell` when none is present. Linux arm64 has no build, so install Chrome/Chromium manually and point `EDGE_CHROME_PATH=/path/to/chrome` at it.
