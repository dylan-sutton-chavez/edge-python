# Edge Python CLI

The Edge Python developer command line interface. Write `.py`, run it, serve it, ship it. You never compile anything: `edge` hosts the Edge Python runtime in a headless Chromium provisioned by `install.sh`, then runs your code against it.

```bash
edge run app.py     # run a script
edge serve          # dev server with live reload
edge repl           # interactive shell
edge test           # run your *_test.py files
edge init my-app    # scaffold a project
edge add network    # add a package to packages.json
edge remove network # remove a package
edge build          # bundle to dist/
edge uninstall      # remove the binary, PATH entry, optionally the bundled browser
```

## Install

```bash
# Prebuilt binary (recommended) — compatible with macOS, Linux and WSL
curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh

# Or from source (any platform with Rust + Cargo)
cargo install --path cli
```

`install.sh` drops the binary at `~/.local/bin/edge`, puts it on `PATH`, and, if no Chrome/Chromium-flavored browser is already reachable, downloads a pinned `chrome-headless-shell` into `~/.cache/edge` and records its path in `EDGE_CHROME_PATH`. Needs `unzip`. Re-run the same line to upgrade. Point `EDGE_CHROME_PATH=/path/to/chrome` at a custom browser to skip the download.

Full command reference, flags, and examples: [edgepython.com/reference/cli](https://edgepython.com/reference/cli).

## Build + test

The CLI is a standalone Cargo workspace under `cli/`. Commands run from there:

```bash
cd cli
cargo clippy --all-targets -- -D warnings # lint
cargo check # type-check
cargo test # run tests (needs any Chrome/Chromium: the bundled shell, a system browser, or EDGE_CHROME_PATH)
```

## License

MIT OR Apache-2.0
