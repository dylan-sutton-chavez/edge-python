/*
Engine orchestrator. Internal to the Worker; consumers use `createWorker` in `src/index.js`.
Lifecycle: `load` once -> many `run` cycles -> `dispose`. Each run instantiates compiler fresh, no state leak.
*/

import { MemoryCache } from '../cache/memory.js';
import { bfsPrefetch } from '../prefetch.js';
import { makeCompilerEnv } from '../env.js';
import { makeRt } from '../rt.js';
import { nativeTable, resetNativeTable } from '../native.js';
import { SOURCE_LIMIT } from '../specs.js';

const TE = new TextEncoder();
const TD = new TextDecoder();

/* Packed status from `run_start` / `run_resume`; mirrors `src/wasm/exports.rs`. */
const STATUS_KIND_SHIFT = 29;
const STATUS_PAYLOAD_MASK = (1 << STATUS_KIND_SHIFT) - 1;
const STATUS_DONE = 0;
const STATUS_PENDING_TIMER = 1;
const STATUS_PENDING_FRAME = 2;
const STATUS_PENDING_EVENT = 3;
const STATUS_ERROR = 4;
const STATUS_PENDING_HOST_CALL = 5;
const STATUS_EXIT = 6; // uncaught SystemExit: clean termination, low 8 bits = exit code
const STATUS_PREEMPTED = 7; // preempt tick; resumes with no host action
const ERR_RUNTIME = 2; // wasm-abi error_kind::RUNTIME, for failed deferred host calls

// Worker-lifetime state
let wasmModule = null;
let compilerExports = null;
let cache = null;
let integrityActive = false;
let loaders = [];
let importsMap = null;
// Resolves run()'s current `await` when a `PendingEvent` wake-up arrives via `pushEvent`.
let eventWaiter = null;
// Events `pushEvent`'d before the VM was ready (no `compilerExports`, or no paused run yet). Drained at the next `PENDING_EVENT` yield.
const pendingEvents = [];
/* Deferred host calls captured by env.host_call_native, keyed by the VM-assigned call_id; drained concurrently in the PENDING_HOST_CALL branch. */
const pendingHostCalls = new Map();
// Back-edges between preempt yields; 0 disables.
let preemptEvery = 0;
let pauseRequested = false;
// True from run entry so mid-boot pause waits.
let running = false;
// Resolves pause() once actually parked.
let pauseAck = null;
// Resolves at resume() to release a preempt hold.
let resumeGate = null;
/* (name, args) => Promise<value>. Set by worker.js (postMessage round-trip) or by a main-thread embedder. */
let hostCallDelegate = null;
/* Host modules resolvable by bare name but loaded on demand; (name) => Promise<exportNames>. */
let loadHostDelegate = null;
let lazyHostNames = [];
// Source/missing caches persist across runs so the BFS skips refetching modules and re-probing 404'd `packages.json` paths on every Run press. Wiped by `clearCache()`.
const fetchedSources = new Map();
const knownMissing = new Set();
/* Synthetic native modules (handlers live on main thread). Re-applied at every `run` since `resetNativeTable` clears them. */
let mainThreadManifests = [];

export async function load({ wasmUrl, integrity = true, loaders: loaderUrls = [], imports = null, version = null, availableHosts = [] }, manifests = []) {
    const t0 = performance.now();
    importsMap = imports;
    lazyHostNames = availableHosts;

    cache = await openCache(integrity);
    integrityActive = cache instanceof MemoryCache ? false : Boolean(integrity);

    if (integrityActive) {
        const stored = await cache.getVersion();
        if (!version || stored !== version) {
            await cache.clear();
            if (version) await cache.setVersion(version);
        }
    }

    loaders = await Promise.all(
        loaderUrls.map(async (url) => (await import(url)).default)
    );

    mainThreadManifests = manifests;

    // Plain fetch, no SRI; the browser decodes any Content-Encoding (br/gzip) before compileStreaming.
    const response = await fetch(wasmUrl);
    if (!response.ok) throw new Error(`fetch failed for '${wasmUrl}' (${response.status})`);
    const wrapped = new Response(response.body, { headers: { 'Content-Type': 'application/wasm' } });
    wasmModule = await WebAssembly.compileStreaming(wrapped);

    return { integrityActive, loadMs: performance.now() - t0 };
}

export async function run(opts) {
    running = true;
    try {
        const payload = TE.encode(opts.src);
        if (payload.length > SOURCE_LIMIT) throw new Error(`source exceeds ${SOURCE_LIMIT} bytes`);
        // REPL inputs keep the interpreter alive in the wasm instance; implies incremental so the instance itself persists too.
        if (opts.repl) {
            return await execute({ ...opts, payload, incremental: true, start: (e, n) => e.repl_eval(n) });
        }
        return await execute({ ...opts, payload, start: (e, n) => e.run_start(n) });
    }
    finally { running = false; }
}

/* Shared run/restore core: instance, host imports, prefetch, then drive `start`. */
async function execute({ src, payload, start, entryDir = '', baseUrl = null, onLine, incremental = false }) {
    if (!wasmModule) throw new Error('engine.load() must be called first');
    entryDir = entryDir.replace(/^(\.\/)+/, ''); // specs never carry ./

    let lockfile = new Map();
    if (integrityActive) {
        try { lockfile = await cache.loadLockfile(); }
        catch { /* lockfile load failure is non-fatal; treat as empty */ }
    }

    /* rt built first (lazy getter) so makeCompilerEnv can decode handles during deferred host calls. */
    const rt = makeRt(() => compilerExports);

    /* Incremental mode reuses the existing wasm instance so module-level state (imports, defs) persists across runs. `onLine` lives in worker.js and is stable, so old env closures still post correctly. */
    let exports;
    if (incremental && compilerExports) {
        exports = compilerExports;
    } else {
        exports = await makeInstance(onLine, lockfile, rt);
    }

    const registerHost = makeRegisterHost(exports);

    /* Both kinds graft `<name> -> mt:<name>` so the bare name resolves; eager ones (programmatic objects) register now, lazy ones (urls) load on first import during prefetch. In incremental mode the native table is preserved, so skip re-registration. */
    const { mainThreadSpecs, augmentedImports } = hostImportMap(registerHost, incremental);

    const writePayload = () => new Uint8Array(exports.memory.buffer).set(payload, exports.src_ptr());
    writePayload();

    await bfsPrefetch(src, exports, lockfile, {
        cache,
        baseUrl,
        entryDir,
        knownMissing,
        importsMap: augmentedImports,
        mainThreadSpecs,
        integrityActive,
        fetchedSources,
        compilerExports: exports,
        rt,
        loaders,
        // Lazy host: fetch export names from the page, then register the mt: stubs here.
        loadHost: (name, url) => {
            if (!loadHostDelegate) throw new Error(`host '${name}' imported but no main-thread loader is wired`);
            return loadHostDelegate(name, url);
        },
        registerHost,
    });

    // Compiler roots the entry's quoted imports at this directory.
    if (exports.set_entry_dir) {
        const dirBytes = TE.encode(entryDir);
        new Uint8Array(exports.memory.buffer).set(dirBytes, exports.src_ptr());
        exports.set_entry_dir(dirBytes.length);
    }

    // `wasm_alloc` during prefetch may have grown memory and detached our view.
    writePayload();

    const t0 = performance.now();
    pendingHostCalls.clear(); // drop any stale captures from a prior run
    const result = await drive(exports, rt, start(exports, payload.length), t0);

    if (integrityActive) {
        try { await cache.saveLockfile(lockfile); }
        catch { /* persistence failure is non-fatal; lockfile lives in-memory until next save */ }
    }

    return result;
}

/* Register a main-thread module at `mt:<name>`: push a stub per export (the real call defers to the page) and tell the compiler its export names. */
const makeRegisterHost = (exports) => (name, exportNames) => {
    const baseId = nativeTable.length;
    for (const fnName of exportNames) {
        const stub = () => {};
        stub.__edge_kind = 'capability';
        stub.__edge_main_thread = true;
        stub.__edge_name = fnName;
        stub.__edge_module = name;
        nativeTable.push(stub);
    }
    const writeBytes = (bytes) => {
        const ptr = exports.wasm_alloc(Math.max(1, bytes.length));
        new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
        return ptr;
    };
    const specBytes = TE.encode(`mt:${name}`);
    const namesBytes = TE.encode(exportNames.join('\n'));
    exports.register_native_module(
        writeBytes(specBytes), specBytes.length,
        writeBytes(namesBytes), namesBytes.length,
        baseId,
    );
};

/* Eager mt: registrations plus the name -> mt:<name> import grafts shared by run() and restoreState(). */
function hostImportMap(registerHost, skipRegistration) {
    const mainThreadSpecs = new Set();
    const augmentedImports = { ...(importsMap || {}) }; // defaults already folded in by the embedder (index.js)
    for (const m of mainThreadManifests) {
        if (!skipRegistration) registerHost(m.name, m.exports);
        mainThreadSpecs.add(`mt:${m.name}`);
        augmentedImports[m.name] = `mt:${m.name}`;
    }
    for (const name of lazyHostNames) {
        if (!mainThreadSpecs.has(`mt:${name}`)) augmentedImports[name] = `mt:${name}`;
    }
    return { mainThreadSpecs, augmentedImports };
}

/* Fresh instance; resets module registry and native table. */
async function makeInstance(onLine, lockfile, rt) {
    const env = makeCompilerEnv({
        getExports: () => compilerExports,
        onLine: onLine ?? (() => {}),
        fetchedSources,
        lockfile,
        integrityActive,
        rt,
        captureHostCall: (id, call) => { pendingHostCalls.set(id, call); },
    });
    const { exports } = await WebAssembly.instantiate(wasmModule, { env });
    compilerExports = exports;
    exports.reset_modules();
    applyPreemptInterval(exports);
    resetNativeTable();
    return exports;
}

function settlePause(parked) {
    const ack = pauseAck;
    pauseAck = null;
    if (ack) ack(parked);
}

/* Service yields until Done / Error / Exit. */
async function drive(exports, rt, status, t0) {
    while (true) {
        const kind = (status >>> STATUS_KIND_SHIFT) & 7;
        if (kind === STATUS_DONE || kind === STATUS_ERROR || kind === STATUS_EXIT) break;
        // Any yield is a park; settle pause() here.
        if (pauseRequested) settlePause(true);
        if (kind === STATUS_PREEMPTED) {
            // Macrotask, so queued postMessage events land.
            await new Promise((r) => setTimeout(r, 0));
            if (pauseRequested) await new Promise((r) => { resumeGate = r; });
        } else if (kind === STATUS_PENDING_TIMER) {
            const deadlineNs = exports.last_yield_deadline_ns();
            const nowNs = BigInt(Date.now()) * 1_000_000n;
            const waitMs = deadlineNs > nowNs ? Number((deadlineNs - nowNs) / 1_000_000n) : 0;
            await new Promise(r => setTimeout(r, waitMs));
        } else if (kind === STATUS_PENDING_FRAME) {
            await new Promise(r => requestAnimationFrame(r));
        } else if (kind === STATUS_PENDING_EVENT) {
            // Drain events buffered before VM was ready. `inject_event` wakes the waiter on the first and queues the rest for later `receive()` calls, no `await` needed.
            let injected = 0;
            while (pendingEvents.length > 0 && injectEvent(pendingEvents[0])) {
                pendingEvents.shift();
                injected++;
            }
            if (injected === 0) {
                await new Promise(r => { eventWaiter = r; });
            }
        } else if (kind === STATUS_PENDING_HOST_CALL) {
            if (pendingHostCalls.size === 0) throw new Error('PENDING_HOST_CALL without captured args (compiler/runtime drift)');
            if (!hostCallDelegate) throw new Error('native deferred but setHostCallDelegate() never set');
            const batch = [...pendingHostCalls];
            pendingHostCalls.clear();
            // a failed call raises only in its own coro, so one bad fetch can't sink the batch
            const outcomes = await Promise.allSettled(batch.map(async ([id, call]) => {
                let rv;
                try {
                    const handle = rt.encodeAny(await hostCallDelegate(call.module, call.name, call.args));
                    rv = exports.set_host_result_by_id(id, handle);
                } catch (e) {
                    rv = exports.set_host_error_by_id(id, ERR_RUNTIME, rt.encodeAny(e?.message ?? String(e)));
                }
                if (rv !== 0) throw new Error(`host-call ${id} delivery returned ${rv} for '${call.module}.${call.name}'`);
            }));
            const drift = outcomes.find((o) => o.status === 'rejected');
            if (drift) throw drift.reason;
        } else {
            // Unknown kind, bail out instead of looping forever.
            break;
        }

        status = exports.run_resume();
    }
    // Run over; a parkless pause must not hang.
    pauseRequested = false;
    settlePause(false);
    // SystemExit: low 8 bits are the exit code, not a buffer length; finish without a traceback.
    if (((status >>> STATUS_KIND_SHIFT) & 7) === STATUS_EXIT) {
        return { out: '', ms: performance.now() - t0, exitCode: status & 0xFF };
    }
    const len = status & STATUS_PAYLOAD_MASK;
    const ms = performance.now() - t0;
    const out = len > 0
        ? TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), len))
        : '';
    return { out, ms };
}

/* Header layout mirrors vm/snapshot.rs `header`. */
function snapshotSource(blob) {
    if (blob.length < 24) throw new Error('not an edge-python snapshot');
    const v = new DataView(blob.buffer, blob.byteOffset, blob.byteLength);
    if (v.getUint32(0, true) !== 0x4E535045) throw new Error('not an edge-python snapshot');
    const len = Number(v.getBigUint64(16, true));
    if (24 + len > blob.length) throw new Error('not an edge-python snapshot');
    return TD.decode(blob.subarray(24, 24 + len));
}

/* Older wasm lacks the export; only preemption needs it. */
function applyPreemptInterval(exports) {
    if (!exports.set_preempt_interval) {
        if (preemptEvery > 0) throw new Error('preemption needs a newer compiler.wasm');
        return;
    }
    exports.set_preempt_interval(preemptEvery);
}

/* Preempt every `n` back-edges; 0 disables. */
export function setPreemptInterval(n) {
    preemptEvery = Math.max(0, n | 0);
    if (compilerExports) applyPreemptInterval(compilerExports);
}

/* Park the run; resolves true once parked. */
export function pause() {
    if (!running) return Promise.resolve(false);
    pauseRequested = true;
    return new Promise((r) => { pauseAck = r; });
}

/* Release a preempt-held run; no-op otherwise. */
export function resume() {
    pauseRequested = false;
    const gate = resumeGate;
    resumeGate = null;
    if (gate) gate();
}

/* Serialize the parked run; throws when none. */
export function saveState() {
    if (!compilerExports) throw new Error('nothing to save: no run has started');
    const len = Number(compilerExports.save_state());
    if (len < 0) throw new Error('nothing to save: the program is not paused');
    return new Uint8Array(compilerExports.memory.buffer, compilerExports.snapshot_ptr(), len).slice();
}

/* Boot from the blob's embedded source, continue from the saved state; resolves like run(). */
export async function restoreState({ blob, onLine }) {
    running = true;
    try {
        const payload = blob instanceof Uint8Array ? blob : new Uint8Array(blob);
        if (payload.length > SOURCE_LIMIT) throw new Error(`snapshot exceeds ${SOURCE_LIMIT} bytes`);
        // The embedded source drives prefetch so restored imports resolve.
        return await execute({ src: snapshotSource(payload), payload, onLine, start: (e, n) => e.restore_state(n) });
    }
    finally { running = false; }
}

/* Parked program's module bindings as JSON. */
export function stateGlobals() {
    if (!compilerExports) return {};
    const len = compilerExports.state_globals();
    return JSON.parse(TD.decode(new Uint8Array(compilerExports.memory.buffer, compilerExports.out_ptr(), len)));
}

/* Parked program's coroutines as JSON. */
export function stateStack() {
    if (!compilerExports) return [];
    const len = compilerExports.state_stack();
    return JSON.parse(TD.decode(new Uint8Array(compilerExports.memory.buffer, compilerExports.out_ptr(), len)));
}

/* Inject directly into the paused VM. Returns false if the VM isn't ready yet (no compilerExports, or no paused run) so callers can buffer. */
function injectEvent(message) {
    if (!compilerExports) return false;
    const bytes = TE.encode(message);
    const ptr = compilerExports.wasm_alloc(bytes.length);
    new Uint8Array(compilerExports.memory.buffer, ptr, bytes.length).set(bytes);
    const status = compilerExports.run_push_event(ptr, bytes.length);
    compilerExports.wasm_free(ptr, bytes.length);
    return status === 0;
}

/* Push a string into the VM's event queue; wakes `receive()`. Buffers if the VM isn't paused on PENDING_EVENT yet, the driver loop drains the buffer at the next yield, so callers never need to know about the VM's readiness window. */
export function pushEvent(message) {
    const msg = String(message);
    if (!injectEvent(msg)) {
        pendingEvents.push(msg);
        return true;
    }
    if (eventWaiter) {
        const w = eventWaiter;
        eventWaiter = null;
        w();
    }
    return true;
}

/* Register the host-call delegate. worker.js wires a postMessage round-trip; no other consumer is supported. */
export function setHostCallDelegate(fn) {
    hostCallDelegate = fn;
}

/* Register the lazy host loader: (name) => Promise<exportNames>. worker.js wires the postMessage round-trip. */
export function setLoadHostDelegate(fn) {
    loadHostDelegate = fn;
}

export function reset() {
    if (compilerExports) compilerExports.reset_modules();
    resetNativeTable();
    pendingHostCalls.clear();
}

export async function clearCache() {
    fetchedSources.clear();
    knownMissing.clear();
    if (cache) await cache.clear();
}

export function dispose() {
    wasmModule = null;
    compilerExports = null;
    cache = null;
    loaders = [];
    importsMap = null;
    fetchedSources.clear();
    knownMissing.clear();
    resetNativeTable();
    pendingHostCalls.clear();
    hostCallDelegate = null;
    loadHostDelegate = null;
    lazyHostNames = [];
    mainThreadManifests = [];
}

async function openCache(integrity) {
    if (!integrity) return new MemoryCache();
    try {
        const { IdbCache } = await import('../cache/idb.js');
        const idb = new IdbCache();
        await idb.open();
        return idb;
    } catch (e) {
        console.warn(
            '[edge-python] integrity:true requested but IndexedDB unavailable; '
            + 'running with in-memory cache. Check worker.integrityActive to detect.',
            e?.message ?? ''
        );
        return new MemoryCache();
    }
}
