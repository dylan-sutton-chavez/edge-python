import { DEFAULT_SYSTEM, DEFAULT_IMPORTS } from './defaults.js';

/* Public entry. `createWorker(opts)` spawns a Web Worker around `engine.js` and returns a proxy whose methods round-trip via postMessage. See README for options. */
export async function createWorker(opts) {
    // Chromium blocks `new Worker(crossOriginUrl)` even with `type:'module'`, cross-origin runtimes need the Blob bootstrap below.
    const workerUrl = new URL('./worker/worker.js', import.meta.url);
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

    /* Lazy system modules, name -> ESM url, imported only when the worker reports the bare name is used. Base defaults sit under user entries, `defaults:false` opts out. */
    const systemUrls = { ...(opts?.defaults !== false ? DEFAULT_SYSTEM : {}), ...(opts?.systemModules || {}) };
    const loadedSystems = new Map(); // name -> export names, memoized across runs
    const loadSystemModule = async (name, manifestUrl) => {
        if (loadedSystems.has(name)) return loadedSystems.get(name);
        // Embedder entries win, manifest-declared systems supply their own url.
        const url = systemUrls[name] ?? manifestUrl;
        if (!url) throw new Error(`no system module registered for '${name}'`);
        const mod = await import(url);
        const factory = mod[name] ?? mod.default;
        const handlers = typeof factory === 'function' ? factory({ pushEvent }) : factory;
        for (const [fnName, handler] of Object.entries(handlers)) {
            mainThreadHandlers[`${name}:${fnName}`] = handler;
        }
        const exports = Object.keys(handlers);
        loadedSystems.set(name, exports);
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
        if (data.type === 'load-system') {
            try {
                const exports = await loadSystemModule(data.name, data.url);
                worker.postMessage({ type: 'load-system-response', reqId: data.reqId, exports });
            } catch (e) {
                worker.postMessage({ type: 'load-system-response', reqId: data.reqId, error: e?.message ?? String(e) });
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

    /* Strip mainThreadModules/systemModules before crossing postMessage, not structured-cloneable / loaded on the page. The worker only needs eager manifests and the lazy system names. */
    const { mainThreadModules: _drop, systemModules: _dropSystems, ...workerOpts } = opts || {};
    /* Fold the std .wasm defaults into imports here so the worker engine stays embedder-neutral, `defaults:false` opts out. */
    const imports = { ...(opts?.defaults !== false ? DEFAULT_IMPORTS : {}), ...(opts?.imports || {}) };
    const ready = await send('load', {
        opts: { ...workerOpts, imports, availableSystems: Object.keys(systemUrls) },
        mainThreadManifests: manifests,
    });

    /* Browser bridges fire `CustomEvent("edge-python-event")` on the global, route the detail to the Worker. Gated on `document` to skip Workers / Deno where this listener has no meaning. */
    if (typeof document !== 'undefined') {
        addEventListener('edge-python-event', (e) => {
            if (typeof e.detail === 'string') pushEvent(e.detail);
        });
    }

    return {
        integrityActive: ready.integrityActive,
        loadMs: ready.loadMs,

        run: (src, runOpts = {}) => send('run', { src, ...runOpts }),
        /* Preempt every `interval` back-edges, 0 disables. */
        setPreemptInterval: (interval) => send('set-preempt-interval', { interval }),
        /* Park the program, resolves true when parked. */
        pause: () => send('pause'),
        /* Continue a program parked by pause(). */
        resume: () => send('resume'),
        /* Snapshot the paused program, throws when none. */
        saveState: () => send('save-state'),
        /* Boot from a blob, resolves like run(). */
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

/* Blob URL inherits the page's origin -> sidesteps Chromium's cross-origin block. The imported module then loads under CORS (Cloudflare Pages OK by default). `Function.toString` keeps the bootstrap as real JS in source. */
function spawnCrossOriginWorker(workerUrl) {
    const source = `(${crossOriginBootstrap.toString()})(${JSON.stringify(workerUrl)});`;
    const blob = new Blob([source], { type: 'application/javascript' });
    const blobUrl = URL.createObjectURL(blob);
    const worker = new Worker(blobUrl, { type: 'module' });
    // Defer revoke a tick, some browsers race it against the module fetch.
    setTimeout(() => URL.revokeObjectURL(blobUrl), 0);
    return worker;
}
