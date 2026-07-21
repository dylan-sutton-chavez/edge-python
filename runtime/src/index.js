/*
Public entry. `createWorker(opts)` spawns a Web Worker around `engine.js` and returns a proxy whose methods round-trip via postMessage. See README for options.
*/

import { DEFAULT_HOST, DEFAULT_IMPORTS } from './defaults.js';

export async function createWorker(opts) {
    // Chromium blocks `new Worker(crossOriginUrl)` even with `type:'module'`; cross-origin runtimes need the Blob bootstrap below.
    const workerUrl = new URL('../worker/worker.js', import.meta.url);
    const sameOrigin = workerUrl.origin === self.location.origin;
    const worker = sameOrigin
        ? new Worker(workerUrl, { type: 'module' })
        : spawnCrossOriginWorker(workerUrl.href);

    let reqIdCounter = 0;
    const pending = new Map();
    let outputHandler = null;

    /* Fire a string into the running script's `receive()` queue. Defined early so main-thread module factories can capture it. */
    const pushEvent = (message) => worker.postMessage({ type: 'push-event', message: String(message) });

    /* Resolve each `mainThreadModules[name]` (factory or object) into a flat handler map keyed `module:name`. */
    const mainThreadHandlers = {};
    const manifests = [];
    for (const [modName, src] of Object.entries(opts?.mainThreadModules || {})) {
        const handlers = typeof src === 'function' ? src({ pushEvent }) : src;
        manifests.push({ name: modName, exports: Object.keys(handlers) });
        for (const [fnName, handler] of Object.entries(handlers)) {
            mainThreadHandlers[`${modName}:${fnName}`] = handler;
        }
    }

    /* Lazy host modules: name -> ESM url, imported only when the worker reports the bare name is used. Base defaults sit under user entries; `defaults:false` opts out. */
    const hostUrls = { ...(opts?.defaults !== false ? DEFAULT_HOST : {}), ...(opts?.hostModules || {}) };
    const loadedHosts = new Map(); // name -> export names, memoized across runs
    const loadHostModule = async (name, manifestUrl) => {
        if (loadedHosts.has(name)) return loadedHosts.get(name);
        // Embedder entries win; manifest-declared hosts supply their own url.
        const url = hostUrls[name] ?? manifestUrl;
        if (!url) throw new Error(`no host module registered for '${name}'`);
        const mod = await import(url);
        const factory = mod[name] ?? mod.default;
        const handlers = typeof factory === 'function' ? factory({ pushEvent }) : factory;
        for (const [fnName, handler] of Object.entries(handlers)) {
            mainThreadHandlers[`${name}:${fnName}`] = handler;
        }
        const exports = Object.keys(handlers);
        loadedHosts.set(name, exports);
        return exports;
    };

    worker.onmessage = async ({ data }) => {
        if (data.type === 'line') {
            if (outputHandler) outputHandler(data.text);
            return;
        }
        if (data.type === 'host-call') {
            const handler = mainThreadHandlers[`${data.module}:${data.name}`];
            if (!handler) {
                worker.postMessage({ type: 'host-call-response', reqId: data.reqId, error: `no main-thread handler for '${data.module}.${data.name}'` });
                return;
            }
            try {
                const value = await handler(...data.args);
                worker.postMessage({ type: 'host-call-response', reqId: data.reqId, value });
            } catch (e) {
                worker.postMessage({ type: 'host-call-response', reqId: data.reqId, error: e?.message ?? String(e) });
            }
            return;
        }
        if (data.type === 'load-host') {
            try {
                const exports = await loadHostModule(data.name, data.url);
                worker.postMessage({ type: 'load-host-response', reqId: data.reqId, exports });
            } catch (e) {
                worker.postMessage({ type: 'load-host-response', reqId: data.reqId, error: e?.message ?? String(e) });
            }
            return;
        }
        const cb = pending.get(data.reqId);
        if (!cb) return;
        pending.delete(data.reqId);
        if (data.type === 'error') cb.reject(new Error(data.message));
        else cb.resolve(data.result);
    };

    worker.onerror = (e) => {
        const err = new Error(e.message || 'worker error');
        for (const cb of pending.values()) cb.reject(err);
        pending.clear();
    };

    const send = (type, payload = {}) => new Promise((resolve, reject) => {
        const reqId = ++reqIdCounter;
        pending.set(reqId, { resolve, reject });
        worker.postMessage({ type, reqId, ...payload });
    });

    /* Strip mainThreadModules/hostModules before crossing postMessage: not structured-cloneable / loaded on the page. The worker only needs eager manifests and the lazy host names. */
    const { mainThreadModules: _drop, hostModules: _dropHosts, ...workerOpts } = opts || {};
    /* Fold the std .wasm defaults into imports here so the worker engine stays embedder-neutral; `defaults:false` opts out. */
    const imports = { ...(opts?.defaults !== false ? DEFAULT_IMPORTS : {}), ...(opts?.imports || {}) };
    const ready = await send('load', {
        opts: { ...workerOpts, imports, availableHosts: Object.keys(hostUrls) },
        mainThreadManifests: manifests,
    });

    /* Browser bridges fire `CustomEvent("edge-python-event")` on the global; route the detail to the Worker. Gated on `document` to skip Workers / Deno where this listener has no meaning. */
    if (typeof document !== 'undefined') {
        addEventListener('edge-python-event', (e) => {
            if (typeof e.detail === 'string') pushEvent(e.detail);
        });
    }

    return {
        integrityActive: ready.integrityActive,
        loadMs: ready.loadMs,

        run: (src, runOpts = {}) => send('run', { src, ...runOpts }),
        /* Snapshot of the paused program as a portable Uint8Array; throws when nothing is paused. */
        saveState: () => send('save-state'),
        /* Boot from a saved blob and continue it; resolves like run() when the program finishes. */
        restoreState: (blob) => send('restore-state', { blob }),
        stateGlobals: () => send('state-globals'),
        stateStack: () => send('state-stack'),
        reset: () => send('reset'),
        clearCache: () => send('clearCache'),
        pushEvent,

        onOutput(handler) { outputHandler = handler; },

        dispose() {
            worker.postMessage({ type: 'dispose' });
            worker.terminate();
            for (const cb of pending.values()) cb.reject(new Error('worker disposed'));
            pending.clear();
        },
    };
}

/* Runs inside the worker. Buffers messages during dynamic import, otherwise `postMessage('load')` dispatches before worker.js installs `self.onmessage` and the first message is lost. */
function crossOriginBootstrap(workerUrl) {
    const buffered = [];
    const enqueue = (event) => buffered.push(event.data);
    self.addEventListener('message', enqueue);
    import(workerUrl).then(() => {
        self.removeEventListener('message', enqueue);
        for (const data of buffered) self.dispatchEvent(new MessageEvent('message', { data }));
    }, (err) => {
        self.postMessage({ type: 'error', message: 'worker bootstrap failed: ' + (err && err.message || err) });
    });
}

/* Blob URL inherits the page's origin -> sidesteps Chromium's cross-origin block; the imported module then loads under CORS (Cloudflare Pages OK by default). `Function.toString` keeps the bootstrap as real JS in source. */
function spawnCrossOriginWorker(workerUrl) {
    const source = `(${crossOriginBootstrap.toString()})(${JSON.stringify(workerUrl)});`;
    const blob = new Blob([source], { type: 'application/javascript' });
    const blobUrl = URL.createObjectURL(blob);
    const worker = new Worker(blobUrl, { type: 'module' });
    // Defer revoke a tick; some browsers race it against the module fetch.
    setTimeout(() => URL.revokeObjectURL(blobUrl), 0);
    return worker;
}
