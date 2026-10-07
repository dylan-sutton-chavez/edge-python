WASM := target/wasm32-unknown-unknown
RUNTIME := --target wasm32-unknown-unknown -p edge-python --lib --no-default-features --features runtime --crate-type cdylib
WASM_FEATURES := --enable-bulk-memory-opt --enable-nontrapping-float-to-int --enable-sign-ext
TSC := npm:typescript@5.9.3/tsc
TREE := $(CURDIR)/_cdn
EXE := $(if $(filter Windows_NT,$(OS)),.exe)

# The CLI embeds the speed compiler, js/dist and the Lucide names, unless the environment points it at copies.
CLI_INPUTS := $(if $(EDGE_COMPILER_WASM),,wasm-cli) $(if $(EDGE_JS_DIST),,js) lucide

CLI_TEST_engine := --bin edge --test cli --test run -- --skip builtin_corpora_mirror_the_web_api
CLI_TEST_network := --test run builtin_corpora_mirror_the_web_api
CLI_TEST_actors := --test actor
CLI_NEEDS_engine := plugin
CLI_NEEDS_ := plugin
CLI_NEEDS_skill := test-skill

MIRI_vm := -p edge-python --test tests vm::
MIRI_snapshot := -p edge-python --test tests snapshot::
MIRI_modules := -p edge-python --test tests modules::
MIRI_parser := -p edge-python --test tests parser::
MIRI_lexer := -p edge-python --test tests lexer::
MIRI_abi := -p edge-python --test tests abi::

unix_only = $(if $(filter Windows_NT,$(OS)),$(error AFL runs on Linux and macOS only))

.PHONY: wasm wasm-cli wasm-ship wasm-small wasm-cli-opt size js lint lint-rust lint-js lint-cli test bench bench-update plugin cli cli-release stage serve browsers test-js test-cli test-skill miri miri-setup fuzz seeds lucide version check

wasm:
	cargo rustc --locked --release $(RUNTIME)

wasm-cli:
	cargo rustc --locked --profile cli $(RUNTIME)

# The small compiler.wasm that ships, and the speed copy the CLI precompiles to native code.
wasm-ship: wasm-small wasm-cli-opt

# Three passes, -Oz, reflatten for a fresh CFG, then a converge sweep. Traps stay, they guard bounds.
wasm-small: export RUSTFLAGS = -Z location-detail=none -Z fmt-debug=none -Z unstable-options -C panic=immediate-abort
wasm-small:
	cargo +nightly rustc --locked --release $(RUNTIME) -Z build-std=std,panic_abort -Z build-std-features=optimize_for_size
	wasm-opt -Oz --converge --generate-global-effects --strip-debug --strip-producers $(WASM_FEATURES) -o $(WASM)/release/compiler.1.wasm $(WASM)/release/compiler.wasm
	wasm-opt --flatten --rereloop -Oz -Oz $(WASM_FEATURES) -o $(WASM)/release/compiler.2.wasm $(WASM)/release/compiler.1.wasm
	wasm-opt -Oz --converge $(WASM_FEATURES) -o $(WASM)/release/compiler.wasm $(WASM)/release/compiler.2.wasm

# A copy beside the speed build, since the bench counts the build before wasm-opt.
wasm-cli-opt: wasm-cli
	wasm-opt -O3 --enable-bulk-memory --enable-mutable-globals --enable-multivalue --enable-reference-types $(WASM_FEATURES) -o $(WASM)/cli/compiler-cli.wasm $(WASM)/cli/compiler.wasm

size:
	deno run --allow-read make/size.ts $(WASM)/release/compiler.wasm

# Strips types to js/dist and bundles the engine as one classic script the page hands the room.
js:
	cd js && deno run -A $(TSC) -p tsconfig.json
	cd js && deno run -A $(TSC) -p tsconfig.worker.json
	cd js && deno bundle --platform browser --format iife src/worker/worker.ts -o dist/worker/bundle.js

lint: lint-rust lint-js

# Shear refuses dependencies declared but unused, then clippy runs for the host and for wasm.
lint-rust:
	cargo shear
	cargo clippy --locked --all-targets -- -D warnings
	cargo clippy --locked --lib --no-default-features --features edge-python/runtime --target wasm32-unknown-unknown -p edge-python -p slugify-mod -- -D warnings
	cargo clippy --locked --lib --no-default-features --target wasm32-unknown-unknown -p edge-python -- -D warnings

# js/src splits main-thread and worker since tsc rejects mixing the dom and webworker libs.
lint-js:
	deno lint js/ make/
	deno check --config js/src/deno.json js/src/index.ts
	deno check --config js/src/worker/deno.json js/src/worker/worker.ts js/src/worker/engine.ts
	deno check make/

lint-cli: $(CLI_INPUTS)
	cd cli && cargo clippy --locked --all-targets -- -D warnings
	cd cli && cargo check --locked

# Coverage leaves method calls unfused and memcheck recounts every slot, so each runs apart.
test:
	cargo test --locked -p edge-python
	cargo test --locked -p edge-python --features coverage coverage
	cargo test --locked --release -p edge-python --features memcheck --test tests vm::test::test_cases

bench: wasm-cli
	cargo run --locked -p bench --profile cli

bench-update: wasm-cli
	cargo run --locked -p bench --profile cli -- --update

plugin:
	cargo build --locked --release --target wasm32-unknown-unknown -p slugify-mod

cli: $(CLI_INPUTS)
	cd cli && cargo build --locked --release

# musl-gcc has no C++, so the static Linux build runs in Alpine, whose libc is musl.
cli-release: $(CLI_INPUTS)
	$(if $(TARGET),,$(error pass TARGET, for example make cli-release TARGET=aarch64-apple-darwin))
	$(if $(findstring linux-musl,$(TARGET)),deno run -A make/musl.ts $(TARGET),cd cli && cargo build --locked --release --target $(TARGET))
	tar -C cli/target/$(TARGET)/release -czf cli/edge-$(TARGET).tar.gz edge$(EXE)

cdn/node_modules/.package-lock.json: cdn/package-lock.json
	npm --prefix cdn ci

stage: cdn/node_modules/.package-lock.json
	npm --prefix cdn run stage -- $(TREE) $(PARTS)

serve: cdn/node_modules/.package-lock.json
	npm --prefix cdn run serve -- $(TREE)

browsers:
	deno run -A npm:playwright install --with-deps chromium

test-js: plugin
	deno test --allow-all js/tests/

# engine runs cli and run minus the network corpus, network runs that corpus, actors and skill theirs.
test-cli: $(CLI_INPUTS) $(CLI_NEEDS_$(SUITE))
	$(if $(filter-out engine network actors skill,$(SUITE)),$(error SUITE is one of engine network actors skill, or none for all))
	$(if $(filter skill,$(SUITE)),,cd cli && cargo test --locked $(CLI_TEST_$(SUITE)))

test-skill: export SKILL_EDGE = $(CURDIR)/cli/target/debug/edge$(EXE)
test-skill: $(CLI_INPUTS)
	cd cli && cargo build --locked
	cargo test --locked -p skill

miri-setup:
	cargo +nightly miri setup

miri: miri-setup
	$(if $(MIRI_$(SUITE)),,$(error SUITE is one of vm snapshot modules parser lexer abi))
	cargo +nightly miri test --locked $(MIRI_$(SUITE))

fuzz:
	$(unix_only)
	cd fuzz && bash ./deploy.sh

seeds:
	$(unix_only)
	cd fuzz && bash ./seeds.sh

# The icon names a docs card can carry, from the Lucide release VERSION names or the latest.
lucide:
	deno run --allow-net --allow-write=target make/lucide.ts $(VERSION)

version:
	$(if $(TAG),,$(error pass TAG, for example make version TAG=v1.0.0))
	deno run --allow-read make/version.ts $(TAG)

check: wasm lint test
