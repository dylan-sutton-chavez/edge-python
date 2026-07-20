/*
Engine orchestrator. Internal to the Worker; consumers use `createWorker` in `src/index.js`.
Lifecycle: `load` once -> many `run` cycles -> `dispose`. Each run instantiates compiler fresh, no state leak.
*/

import { MemoryCache } from '../src/cache/memory.js';
import { bfsPrefetch } from '../src/prefetch.js';
import { makeCompilerEnv } from '../src/env.js';
import { makeRt } from '../src/rt.js';
import { nativeTable, resetNativeTable } from '../src/native.js';
import { SOURCE_LIMIT } from '../src/specs.js';

const TE = new TextEncoder();
const TD = new TextDecoder();

/* Packed status from `run_start` / `run_resume`; mirrors `compiler/src/main/exports.rs`. */
const STATUS_KIND_SHIFT = 29;
const STATUS_PAYLOAD_MASK = (1 << STATUS_KIND_SHIFT) - 1;
const STATUS_DONE = 0;
const STATUS_PENDING_TIMER = 1;
const STATUS_PENDING_FRAME = 2;
const STATUS_PENDING_EVENT = 3;
const STATUS_ERROR = 4;
const STATUS_PENDING_HOST_CALL = 5;
const STATUS_EXIT = 6; // uncaught SystemExit: clean termination, low 8 bits = exit code
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

export async function run({ src, entryDir = '', baseUrl = null, onLine, incremental = false }) {
    if (!wasmModule) throw new Error('engine.load() must be called before run()');

    const srcBytes = TE.encode(src);
    if (srcBytes.length > SOURCE_LIMIT) throw new Error(`source exceeds ${SOURCE_LIMIT} bytes`);

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
        const env = makeCompilerEnv({
            getExports: () => compilerExports,
            onLine: onLine ?? (() => {}),
            fetchedSources,
            lockfile,
            integrityActive,
            rt,
            captureHostCall: (id, call) => { pendingHostCalls.set(id, call); },
        });
        ({ exports } = await WebAssembly.instantiate(wasmModule, { env }));
        compilerExports = exports;
        exports.reset_modules();
        resetNativeTable();
    }

    const writeBytes = (bytes) => {
        const ptr = exports.wasm_alloc(Math.max(1, bytes.length));
        new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
        return ptr;
    };

    /* Register a main-thread module at `mt:<name>`: push a stub per export (the real call defers to the page) and tell the compiler its export names. */
    const registerHost = (name, exportNames) => {
        const baseId = nativeTable.length;
        for (const fnName of exportNames) {
            const stub = () => {};
            stub.__edge_kind = 'capability';
            stub.__edge_main_thread = true;
            stub.__edge_name = fnName;
            stub.__edge_module = name;
            nativeTable.push(stub);
        }
        const specBytes = TE.encode(`mt:${name}`);
        const namesBytes = TE.encode(exportNames.join('\n'));
        exports.register_native_module(
            writeBytes(specBytes), specBytes.length,
            writeBytes(namesBytes), namesBytes.length,
            baseId,
        );
    };

    /* Both kinds graft `<name> -> mt:<name>` so the bare name resolves; eager ones (programmatic objects) register now, lazy ones (urls) load on first import during prefetch. In incremental mode the native table is preserved, so skip re-registration. */
    const mainThreadSpecs = new Set();
    const augmentedImports = { ...(importsMap || {}) }; // defaults already folded in by the embedder (index.js)
    for (const m of mainThreadManifests) {
        if (!incremental) registerHost(m.name, m.exports);
        mainThreadSpecs.add(`mt:${m.name}`);
        augmentedImports[m.name] = `mt:${m.name}`;
    }
    for (const name of lazyHostNames) {
        if (!mainThreadSpecs.has(`mt:${name}`)) augmentedImports[name] = `mt:${name}`;
    }

    const writeSrc = () => new Uint8Array(exports.memory.buffer).set(srcBytes, exports.src_ptr());
    writeSrc();

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

    // `wasm_alloc` during prefetch may have grown memory and detached our src view.
    writeSrc();

    // Driver loop: `run_start` then `run_resume` after each host wake-up until Done / Error.
    const t0 = performance.now();
    pendingHostCalls.clear(); // drop any stale captures from a prior run
    let status = exports.run_start(srcBytes.length);
    while (true) {
        const kind = (status >>> STATUS_KIND_SHIFT) & 7;
        if (kind === STATUS_DONE || kind === STATUS_ERROR || kind === STATUS_EXIT) break;
        if (kind === STATUS_PENDING_TIMER) {
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
    // SystemExit: low 8 bits are the exit code, not a buffer length; finish without a traceback.
    if (((status >>> STATUS_KIND_SHIFT) & 7) === STATUS_EXIT) {
        return { out: '', ms: performance.now() - t0, exitCode: status & 0xFF };
    }
    const len = status & STATUS_PAYLOAD_MASK;
    const ms = performance.now() - t0;
    const out = len > 0
        ? TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), len))
        : '';

    if (integrityActive) {
        try { await cache.saveLockfile(lockfile); }
        catch { /* persistence failure is non-fatal; lockfile lives in-memory until next save */ }
    }

    return { out, ms };
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
        const { IdbCache } = await import('../src/cache/idb.js');
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
