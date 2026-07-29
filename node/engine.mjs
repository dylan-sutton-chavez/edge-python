// engine.mjs
//
// A from-scratch Node.js host for EdgePython's `compiler.wasm`, reimplementing
// the same `env.*` import contract the official browser runtime uses, so you
// can skip the `edge` CLI's bundled headless Chromium entirely.
//
// This is NOT an official EdgePython artifact. It was reverse-engineered by
// reading the real runtime source (`runtime/src/env.js`, `runtime/worker/engine.js`
// in dylan-sutton-chavez/edge-python) rather than the docs prose, because the
// docs only name the four `env.*` imports without full signatures.
//
// Supported:
//   - one-shot script execution (`run_start` / `run_resume`)
//   - incremental REPL execution (`repl_eval`, same instance kept alive)
//   - print() output, wall-clock time, SystemExit codes, tracebacks
//   - optional "preloaded sources" map for simple quoted-relative imports
//     (`from "./foo.py" import x`) that you supply yourself
//
// NOT implemented (out of scope for a "simple REPL harness"):
//   - packages.json / bare-name std package resolution (math, json, network, ...)
//   - the BFS prefetch + SRI/lockfile machinery the browser runtime uses
//   - Path A `.wasm` native plugin loading (wasm-pdk modules)
//   - Path B/C host capabilities (dom, fs, storage, time, network builtins)
//   - receive()/events, timers, coropossible host callbacks
//
// Scripts that don't import anything, or only import from sources you preload,
// work fully. Anything needing the package registry will raise inside the VM
// (a real, catchable error) rather than silently doing nothing.

import { readFile } from 'node:fs/promises';

const TE = new TextEncoder();
const TD = new TextDecoder();

// Mirrors compiler/src/main/exports.rs packed status word (see engine.js in
// the real runtime, which documents this layout in a comment).
const STATUS_KIND_SHIFT = 29;
const STATUS_PAYLOAD_MASK = (1 << STATUS_KIND_SHIFT) - 1;
const STATUS = Object.freeze({
  DONE: 0,
  PENDING_TIMER: 1,
  PENDING_FRAME: 2,
  PENDING_EVENT: 3,
  ERROR: 4,
  PENDING_HOST_CALL: 5,
  EXIT: 6,
  PREEMPTED: 7,
});
const ERR_RUNTIME = 2; // wasm-abi error_kind::RUNTIME

const SOURCE_LIMIT = 1 << 20; // mirrors runtime/src/specs.js

export class EdgeRuntimeError extends Error {}

/**
 * Load and compile compiler.wasm from disk.
 * @param {string} wasmPath
 */
export async function loadCompiler(wasmPath) {
  const bytes = await readFile(wasmPath);
  return WebAssembly.compile(bytes);
}

/**
 * Create a fresh instance of compiler.wasm with the host imports wired up.
 *
 * @param {WebAssembly.Module} wasmModule
 * @param {object} opts
 * @param {(line: string) => void} opts.onLine - called on every print() flush
 * @param {Map<string, Uint8Array>} [opts.sources] - preloaded import sources,
 *        keyed by the exact spec string the script imports (e.g. "./foo.py").
 *        Left empty, any import will raise inside the script.
 */
export async function makeInstance(wasmModule, { onLine, sources = new Map() } = {}) {
  let exports; // populated below; env closures capture it by reference

  const readStr = (ptr, len) => TD.decode(new Uint8Array(exports.memory.buffer, ptr, len));
  const setU32 = (ptr, v) => new DataView(exports.memory.buffer).setUint32(ptr, v, true);

  const stashError = (message) => {
    const bytes = TE.encode(message);
    const ptr = exports.wasm_alloc(bytes.length);
    new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
    exports.host_edge_throw(ERR_RUNTIME, ptr, bytes.length);
    exports.wasm_free(ptr, bytes.length);
  };

  const env = {
    host_print: (ptr, len) => onLine(readStr(ptr, len)),

    // No native modules / capabilities registered in this shim: any call
    // reaching here means the script imported something we don't provide.
    host_call_native: (id, _call_id, _argv_ptr, _argc, _out_ptr) => {
      stashError(`native call id ${id}: no native modules are registered in this Node shim`);
      return 1;
    },

    // Wall-clock ns as BigInt; the guest expects an i64.
    host_now_ns: () => BigInt(Date.now()) * 1_000_000n,

    // Serves preloaded sources only. Real package resolution (packages.json,
    // bare-name std packages, SRI, lockfiles) is not implemented.
    host_fetch_bytes: (specPtr, specLen, _hashPtr, outLenPtr) => {
      const spec = readStr(specPtr, specLen);
      const bytes = sources.get(spec);
      if (bytes === undefined) {
        setU32(outLenPtr, 0);
        return 0;
      }
      const ptr = exports.wasm_alloc(bytes.length);
      new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
      setU32(outLenPtr, bytes.length);
      return ptr;
    },
  };

  const instance = await WebAssembly.instantiate(wasmModule, { env });
  exports = instance.exports;
  exports.reset_modules();
  return exports;
}

function writeSource(exports, srcText) {
  const payload = TE.encode(srcText);
  if (payload.length > SOURCE_LIMIT) {
    throw new EdgeRuntimeError(`source exceeds ${SOURCE_LIMIT} bytes`);
  }
  new Uint8Array(exports.memory.buffer).set(payload, exports.src_ptr());
  return payload;
}

/**
 * Drive a run to completion, servicing the yield kinds we support.
 * Mirrors `drive()` in the real engine.js, minus browser-only yield kinds.
 */
async function drive(exports, status) {
  for (;;) {
    const kind = (status >>> STATUS_KIND_SHIFT) & 7;

    if (kind === STATUS.DONE || kind === STATUS.ERROR || kind === STATUS.EXIT) break;

    if (kind === STATUS.PREEMPTED) {
      await new Promise((r) => setTimeout(r, 0));
    } else if (kind === STATUS.PENDING_TIMER) {
      const deadlineNs = exports.last_yield_deadline_ns();
      const nowNs = BigInt(Date.now()) * 1_000_000n;
      const waitMs = deadlineNs > nowNs ? Number((deadlineNs - nowNs) / 1_000_000n) : 0;
      await new Promise((r) => setTimeout(r, waitMs));
    } else if (kind === STATUS.PENDING_FRAME || kind === STATUS.PENDING_EVENT || kind === STATUS.PENDING_HOST_CALL) {
      // No requestAnimationFrame, event queue, or native-module dispatch in
      // this minimal harness. Fail loudly instead of hanging forever.
      throw new EdgeRuntimeError(
        `script parked on an unsupported host feature (status kind ${kind}); ` +
        `this Node shim only supports plain script execution, no async/host capabilities`
      );
    } else {
      throw new EdgeRuntimeError(`unknown status kind ${kind} (compiler.wasm ABI drift?)`);
    }

    status = exports.run_resume();
  }

  if (((status >>> STATUS_KIND_SHIFT) & 7) === STATUS.EXIT) {
    return { kind: 'exit', code: status & 0xff };
  }
  if (((status >>> STATUS_KIND_SHIFT) & 7) === STATUS.ERROR) {
    const len = status & STATUS_PAYLOAD_MASK;
    const message = len > 0 ? TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), len)) : '';
    return { kind: 'error', message };
  }
  const len = status & STATUS_PAYLOAD_MASK;
  const out = len > 0 ? TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), len)) : '';
  return { kind: 'done', out };
}

/**
 * Run a whole script once (`edge run` equivalent). Fresh VM state.
 */
export async function runScript(exports, srcText) {
  const payload = writeSource(exports, srcText);
  return drive(exports, exports.run_start(payload.length));
}

/**
 * Run one REPL line against a persistent VM (`edge repl` equivalent).
 * Call this repeatedly against the *same* `exports` to keep definitions,
 * imports, and mutations alive across prompts.
 */
export async function replEval(exports, lineText) {
  const payload = writeSource(exports, lineText);
  return drive(exports, exports.repl_eval(payload.length));
}

export { STATUS };
