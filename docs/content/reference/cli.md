---
title: "Command line interface"
description: "The Edge Python developer command line interface (CLI): run, serve, repl, init, package management, and build."
---

The `edge` developer CLI. Write `.py`, run it, serve it, ship it. You never compile anything yourself. `run`, `repl`, and `test` execute in the built-in [native engine](/reference/native) by default, in-process with millisecond startup; the `--web` flag hosts the [Edge Python runtime](/getting-started/what-it-is#where-it-runs) in a headless Chromium instead, for scripts that need the browser (`dom`, `frame()`, sockets).

```bash
edge run app.py     # run a script
edge serve          # dev server with live reload
edge repl           # interactive shell
edge test           # run *_test.py files
edge init my-app    # scaffold a project
edge add network    # add a package to packages.json
edge remove network # remove a package from packages.json
edge build          # bundle to dist/
edge uninstall      # remove the binary, PATH entry, optionally the bundled browser
```

Under `--web` the browser runtime does the actual work and `edge` is the loop around it: it launches headless Chromium, serves the runtime alongside your code, runs everything there, and streams output back to your terminal. `edge serve` always targets your own browser, with live reload.

## Install

```bash
# Prebuilt binary (recommended), compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

# Or from source (any platform with Rust and Cargo)
cargo install --path cli
```

`install.sh` drops the binary at `~/.local/bin/edge` and appends that directory (plus `EDGE_CHROME_PATH`, when the bundled browser is downloaded) to your `~/.bashrc` or `~/.zshrc` unless the file already has it. Open a new shell (or `source` the file it printed) and `edge --version` should work. Re-run the same `curl … | sh` line any time to upgrade. To remove everything: `curl -fsSL https://cdn.edgepython.com/cli/uninstall.sh | sh` (non-interactive: it leaves the bundled browser cache in place; `edge uninstall` asks before removing it).

On Linux the installer probes your libc and picks the matching build: glibc 2.28+ gets the gnu binary (std `.so` plugins load natively), anything older or musl-based gets a fully static fallback. `install.sh` also provisions a headless browser when none is reachable — set `EDGE_NO_BROWSER=1` to skip it on servers that only use the native engine; details and overrides in [Bring your own browser](#bring-your-own-browser).

## `edge run`: run a Python file

Runs a script and streams its output to the terminal. Bare imports resolve through [`packages.json`](/reference/imports#packagesjson); quoted relative imports load from your project files, resolved against the script's own directory (for a script outside the working directory — an absolute path or `../` — they resolve against the working directory instead). Uncaught errors print a traceback to stderr and exit with code 1.

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

Flags: `--packages <file>` (custom manifest), plus the native-engine flags `--events`, `--save-state`, `--restore-state`, and `--preempt` ([native engine](/reference/native#run-flags); they reject `--web`). With no path, `edge run` reads from stdin if it is piped (`cat hello.py | edge run`). It errors out if stdin is a terminal. With a path, piped stdin feeds [`input()`](/reference/builtins#input) one line per call.

## `edge serve`: local dev server

A dev server for browser apps. Serves your project directory and reloads the page on any file change via an injected polling client.

```text
$ edge serve
  http://localhost:5173
  watching .
```

Flags: `--port <n>` (default `5173`), `--open` (open the browser), `--host <addr>` (bind address, default `127.0.0.1`).

`--host 0.0.0.0` exposes the server on your LAN — the banner adds the network URL, so you can open the app from a phone on the same network:

```text
$ edge serve --host 0.0.0.0
  http://localhost:5173
  http://192.168.1.34:5173
  watching .
```

## `edge repl`: interactive shell

An interactive Edge Python shell for quick experiments.

```text
$ edge repl
Edge Python 0.1.0  ·  .reset to start fresh  ·  .exit, Ctrl+C or Ctrl+D to quit
>>> from math import sqrt, pi
>>> print(sqrt(2))
1.4142135623730951
>>> print([n * n for n in range(5)])
[0, 1, 4, 9, 16]
>>> .exit
```

History (arrow keys) is supported. Each line runs as one input, so compound statements go on a single line (`def double(n): return n * 2`). `.exit`, `Ctrl+C`, or `Ctrl+D` quit. `.reset` wipes the session state and clears the screen. Expression results are not auto-printed. Use `print()` explicitly.

The worker keeps one interpreter alive across prompts: each input compiles and runs **once**, and imports, definitions, and mutations persist in place. Side effects never re-fire, and an input that raises keeps the effects it made before the error.

## `edge test`: test runner

Discovers `*_test.py` files (recursively, skipping `dist/` and hidden directories), runs each in a fresh interpreter inside one shared browser session, and prints a verdict per file. `edge test path/` narrows discovery to a directory; `edge test file.py` runs one file.

```text
my-app/
├─ packages.json
├─ main.py
├─ main_test.py
├─ lib/
│  ├─ parse.py
│  ├─ fixtures.py
│  └─ parse_test.py
└─ dist/
```

`edge test` from the root runs both test files; `edge test lib/` scopes discovery to that subtree. Each file's quoted imports resolve from its own directory (`"./parse.py"`, `"../lib/parse.py"`), clamped at the project root. Bare names (`test`, `math`, the rest of the [registry](/reference/packages)) resolve everywhere. State never leaks between files: every file starts in a fresh interpreter.

```text
$ edge test
PASS - adds
1 passed, 0 failed
  (successful) main_test.py
PASS - parses
1 passed, 0 failed
  (successful) lib/parse_test.py

  2/2 files passed · 1.6s
```

Test files declare tests with the [`test` package](/reference/packages#test) and don't need to call `run()` — the runner drives it after the file loads:

```python
from test import test

@test("adds")
def t_add():
    assert 1 + 1 == 2
```

A file that calls `run()` itself also works: either way its `SystemExit` code is the file's verdict, so the reported result never depends on parsing printed output. A file that registers no tests fails.

Exit codes: `0` every file passed, `1` a file failed or no `*_test.py` was found, `2` the browser session could not start.

## `edge init`: scaffold a workspace

Scaffolds a ready-to-run project: an entry script, an HTML host page, and a manifest.

```text
$ edge init my-app
  created my-app/
    ├─ index.html
    ├─ main.py
    └─ packages.json

  cd my-app && edge serve
```

`--bare` skips `index.html` for script-only projects.

## `edge add` / `edge remove`: package manager

Manage [`packages.json`](/reference/imports#packagesjson) by name. `edge` knows the official std (`json`, `re`, `math`, `struct`, `test`) and host (`dom`, `network`, `storage`, `time`) packages, so you don't paste URLs. Most std packages are `.wasm`. `test` is pure Edge Python, so it resolves to `test.py`. See [Official packages](/reference/packages) for the full catalog.

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

Bundles your app into a self-contained `dist/` for offline use or self-hosting. It vendors the runtime, the `compiler.wasm`, your scripts, and every package your scripts import, so nothing is fetched at runtime.

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

Removes the binary and its `PATH` entry. Asks before removing the bundled `chrome-headless-shell` cache (never touches system Chromium). The `uninstall.sh` one-liner in [Install](#install) does the same non-interactively, leaving the cache in place.

## Global flags

| Flag | Effect |
|------|--------|
| `--packages <file>` | Use a specific manifest instead of `./packages.json` (read by `run`, `repl`, `test`, `add`, `remove`, `build`) |
| `--web` | Drive the browser runtime instead of the in-process [native engine](/reference/native) (`run`, `repl`, `test`) |
| `--version` / `-V` | Print version |

`Ctrl+C` cancels any running command cleanly.

## Bring your own browser

`edge` uses, in order:

1. `EDGE_CHROME_PATH` if set.
2. The bundled `chrome-headless-shell` in `~/.cache/edge` (override the root with `EDGE_CHROME_DIR`).
3. A system `google-chrome` / `chromium` / `chrome` on `PATH` (or the `CHROME` env var).
4. Playwright's Chromium, if installed.

`install.sh` downloads `chrome-headless-shell` when none is present. Linux arm64 has no build, so install Chrome/Chromium manually and point `EDGE_CHROME_PATH=/path/to/chrome` at it.
