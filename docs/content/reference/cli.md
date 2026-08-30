---
title: "Command line interface"
description: "The edge CLI: run, serve, repl, test, init, package management, build, actor, and uninstall."
---

The `edge` CLI runs on macOS, Linux, and WSL. `run`, `repl`, and `test` execute in the built-in [native engine](/reference/modules#the-native-engine) by default, in-process with millisecond startup. The `--web` flag drives the browser runtime in headless Chromium instead, for scripts that need the browser (`dom`, `frame()`, sockets). Under `--web` the CLI is the loop around the browser: it launches Chromium, serves your code, and streams output back to the terminal.

```bash
edge run app.py        # run a script, a .edge, or stdin
edge build             # pack a standalone .edge (--bundle, --web)
edge actor actor.yml # run a pool of cooperative actors
edge serve             # dev server with live reload
edge repl              # interactive shell
edge test              # run *_test.py files
edge init my-app       # scaffold a project
edge add network       # add a package to packages.json
edge remove network    # remove a package from packages.json
edge uninstall         # remove the binary, PATH entry, optionally the bundled browser
```

## Install

```bash
# Prebuilt binary (recommended)
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

# Or from source (any platform with Rust and Cargo)
cargo install --path cli
```

`install.sh` drops the binary at `~/.local/bin/edge` and appends that directory (plus `EDGE_CHROME_PATH`, when it downloads the bundled browser) to your `~/.bashrc` or `~/.zshrc` unless already present. Open a new shell and `edge --version` should work. Re-run the same line to upgrade.

Linux needs glibc 2.17 or newer, the floor the prebuilt binaries are linked against. The installer checks up front and tells you to build from source when the host is older or musl-based, rather than installing a binary that cannot load native plugins. The installer also downloads `chrome-headless-shell` when no browser is reachable. Set `EDGE_NO_BROWSER=1` to skip that on servers that only use the native engine.

## `edge run`: run a script

Runs a script and streams its output to the terminal. Bare imports resolve through [`packages.json`](/reference/modules#packagesjson). Relative imports resolve against the importing file. Uncaught errors print a traceback to stderr and exit 1.

```text
$ edge run broken.py
before
error: ZeroDivisionError: division by zero
  --> broken.py:2:1
  |
2 | x = 1 / 0
  | ^
```

`raise SystemExit(code)` with no argument or an integer exits cleanly with that code and no traceback. A string argument surfaces as a regular error and exits 1.

With no path, `edge run` reads the script from piped stdin (`cat app.py | edge run`) and errors when stdin is a terminal. `-c <code>` runs inline code instead (`edge run -c 'print(1)'`). With a path or `-c`, piped stdin instead feeds [`input()`](/reference/builtins) one line per call. A packed `.edge` or `.package` also runs here, `edge run app.edge` unpacks and runs it exactly as `./app.edge` would.

Flags: the [native-engine flags](/reference/modules#run-flags) `--events`, `--save-state`, `--restore-state`, and `--preempt`. They are native-only and combining any of them with `--web` is an error.

## `edge serve`: local dev server

Serves the current directory for browser apps and reloads the page on any file change, via a small polling client injected into served HTML.

```text
$ edge serve
  http://localhost:5173
  watching .
```

Flags: `--port <n>` (default `5173`), `--host <addr>` (default `127.0.0.1`), `--open` (open a browser). With `--host 0.0.0.0` the banner adds your LAN URL, so you can open the app from a phone on the same network.

## `edge repl`: interactive shell

```text
$ edge repl
Edge Python 0.1.0  ·  .reset to start fresh  ·  .exit, Ctrl+C or Ctrl+D to quit
>>> from math import sqrt
>>> print(sqrt(2))
1.4142135623730951
```

One interpreter stays alive across prompts: imports, definitions, and mutations persist, and an input that raises keeps the effects it made before the error. Each line is one input, so compound statements go on a single line (`def double(n): return n * 2`). Expression results are not auto-printed, use `print()`. Arrow-key history works within the session and is not saved to disk. `.reset` wipes the session state, `.exit` quits, and `Ctrl+C` or `Ctrl+D` also quits the shell.

## `edge test`: test runner

Discovers `*_test.py` files recursively (skipping `dist/` and hidden directories), runs each in a fresh interpreter, and prints a verdict per file. A directory argument narrows discovery to that subtree, a file argument runs just that file.

```text
$ edge test
PASS - adds
1 passed, 0 failed
  (successful) main_test.py
PASS - parses
1 passed, 0 failed
  (successful) lib/parse_test.py

  2/2 files passed · 0.0s
```

Test files declare tests with the [`test` package](/packages/std/test) and do not need to call `run()`. The runner drives it after the file loads and reads the verdict from the file's `SystemExit` code, never from parsed output. A file that registers no tests fails. State never leaks between files.

Exit codes: `0` when every file passed, `1` when a file failed or no `*_test.py` was found, `2` when the engine session could not start.

## `edge init`: scaffold a project

Creates a ready-to-serve project. With no argument it scaffolds the current directory.

```text
$ edge init my-app
  created my-app/
    ├─ index.html
    ├─ main.py
    └─ packages.json

  cd my-app && edge serve
```

`--bare` skips `index.html` for script-only projects.

## `edge add` / `edge remove`: package management

Edits [`packages.json`](/reference/modules#packagesjson) by name. The CLI knows the official packages, std (`json`, `re`, `math`, `struct`, `test`) and system (`dom`, `network`, `storage`, `time`), so you never paste URLs. Std entries go to `imports` (`.wasm` URLs, except the script-only `test` which resolves to `test.py`), system entries go to `system`. The full catalog is in [Modules](/reference/modules#standard-packages).

```text
$ edge add math network
  + math       std
  + network    system

  updated packages.json
```

Point a name at a custom URL with `edge add foo=https://example.com/foo.wasm`. The kind is inferred from the URL: `.wasm` and `.py` mean std, anything else means system. `edge remove` deletes entries the same way.

## `edge build`: pack the app

Packs the project and its imports into one artifact, in one of three modes for three targets.

| Command | Output | Runs on |
|---------|--------|---------|
| `edge build` | a standalone `.edge` binary | any host of the same OS, nothing installed |
| `edge build --bundle` | a lightweight `.package` | a host that already has `edge`, or a pool |
| `edge build --web` | a self-contained `dist/` | any browser |

The default `.edge` is this `edge` binary with the project appended, so `./app.edge` runs it directly and it honors the run flags `--save-state`, `--restore-state`, `--preempt`, and `--events`. The `.package` carries only the code and its custom imports, since the runtime is already present where it lands. `--web` vendors the browser runtime, `compiler.wasm`, and every package into `dist/`, rewriting `packages.json` to the vendored paths. Std packages resolve by name at run time, so an `.edge` or `.package` needs no network for them.

```text
$ edge build

  packed app.edge (3 files)
  4.20 MB

  run  ./app.edge   flags  --save-state --restore-state --preempt --events
```

Flags: `--out <path>` (mode-specific default), `--bundle`, `--web`.

## `edge actor`: actor pool

Runs many edge-python programs as cooperative actors over a few threads, described by a `actor.yml`. See [Actors](/reference/actors) for the manifest, groups, server, and untrusted code.

```bash
edge actor actor.yml
```

## `edge uninstall`

Removes the binary and its `PATH` entry, and asks before removing the bundled `chrome-headless-shell` cache. System browsers are never touched. The non-interactive equivalent is `curl -fsSL https://cdn.edgepython.com/cli/uninstall.sh | sh`, which leaves the browser cache in place.

## Global flags

| Flag | Effect |
|------|--------|
| `--packages <file>` | Use a specific manifest instead of `./packages.json` |
| `--web` | Drive the browser runtime instead of the native engine (`run`, `repl`, `test`) |
| `--version`, `-v` | Print the version |
| `--help`, `-h` | Print the command list |

`Ctrl+C` cancels a running command with exit code 130.

## Bring your own browser

For `--web`, `edge` uses, in order:

1. `EDGE_CHROME_PATH`, when set.
2. The bundled `chrome-headless-shell` under `~/.cache/edge` (override the root with `EDGE_CHROME_DIR`).
3. A system Chrome, Chromium, or Edge on `PATH` (or the `CHROME` env var).
4. Playwright's Chromium, when installed.

`install.sh` downloads `chrome-headless-shell` when none of these is present. There is no Linux arm64 build, so on that platform install Chrome or Chromium manually and point `EDGE_CHROME_PATH` at it.
