# How to Contribute to Edge Python

Thanks for your interest in contributing to Edge. This document outlines some recommendations on how to contribute.

## Issues and Feature Requests

Please provide a failing example if possible to help with issue reproduction.

## Pull Requests

For large changes, please try reaching out to the Edge maintainers via GitHub Issues or Email to ensure that the change can be accepted once it is ready.

Run the following commands before sending a pull request to ensure code quality:

- `cargo wasm` Build the `compiler.wasm`.
- `cargo test --release` Run the compiler test suite.
- `cargo clippy --all-targets -- -D warnings` Lint the Rust code.
- `cargo clippy --lib --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings` Lint the wasm build.
- `cargo clippy --lib --features native -- -D warnings` Lint the native engine module.
- `cargo shear` Detect unused dependencies.
- For significant changes, execute the [fuzzer](https://edgepython.com/implementation/fuzzing/) to check for new crashes or performance regressions.

The test suite (`tests/`, fixtures in `tests/cases/vm.json`) runs every case under `Limits::sandbox()`, not the default `none()`, so budget, heap, and call-depth regressions surface as a `MemoryError` or `RecursionError` assertion instead of a hang. Every fixture must stay within the sandbox budget.

*Other packages have their own build and test setup. See the repository layout section of the root README for the per-package commands.*

`cli/` tests hit the CDN runtime by default; `EDGE_RUNTIME_DIR` and `EDGE_COMPILER_WASM` swap in local copies for end-to-end validation before a deploy:

```bash
cargo wasm && cd cli
EDGE_RUNTIME_DIR=../web EDGE_COMPILER_WASM=../target/wasm32-unknown-unknown/release/compiler.wasm cargo test
```

A CI job will be run by the maintainer after the PR has been created.

PRs that introduce new behavior without test coverage, or that update documentation without reflecting the actual code change, will not be accepted.

## Comments and Docs

Keep comments minimal. One line, at most one per block, deleted when redundant. Match the length of the docs you edit rather than expanding them. No colons, semicolons, or em-dashes in comment or doc prose. No file-header comment or docstring at the top of a file.

Changes that alter language, CLI, or package behavior must update `skill/SKILL.md` to match. Its examples are executable cells, so `cargo test -p skill` must stay green.
