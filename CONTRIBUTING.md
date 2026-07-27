# How to Contribute to Edge Python

Thanks for your interest in contributing to Edge. This document outlines some recommendations on how to contribute.

## Issues and Feature Requests

Please provide a failing example if possible to help with issue reproduction.

## Pull Requests

For large changes, please try reaching out to the Edge maintainers via GitHub Issues or Email to ensure that the change can be accepted once it is ready.

Run the following commands before sending a pull request to ensure code quality:

- `cargo wasm` Build the `compiler.wasm`.
- `cargo test --release --no-default-features` Run the compiler test suite.
- `cargo clippy --all-targets --no-default-features -- -D warnings` Lint the Rust code.
- `cargo clippy --lib --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings` Lint the wasm build.
- `cargo shear` Detect unused dependencies.
- For significant changes, execute the [fuzzer](https://edgepython.com/implementation/fuzzing/) to check for new crashes or performance regressions.

*Other packages have their own build and test setup — see the `README.md` in the relevant path. Code comments must be a single line of fewer than 30 words; if a change is too large, add a short section to the corresponding page under `./docs/content` instead.*

`cli/` tests hit the CDN runtime by default; `EDGE_RUNTIME_DIR` and `EDGE_COMPILER_WASM` swap in local copies for end-to-end validation before a deploy:

```bash
cargo wasm && cd cli
EDGE_RUNTIME_DIR=../js EDGE_COMPILER_WASM=../target/wasm32-unknown-unknown/release/compiler.wasm cargo test
```

A CI job will be run by the maintainer after the PR has been created.

PRs that introduce new behavior without test coverage, or that update documentation without reflecting the actual code change, will not be accepted.
