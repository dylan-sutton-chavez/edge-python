import { DEFAULT_SYSTEM, DEFAULT_IMPORTS } from './defaults.ts';
import type { EdgeValue } from './rt.ts';
import { errMsg } from './util.ts';
import type { MainThreadManifest, RunOpts, ExecResult, WorkerRequest, WorkerMessage } from './protocol.ts';

/* A page-side module handed to `mainThreadModules`, either a flat handler map or a factory that receives `{ pushEvent }`. User-supplied handlers have arbitrary signatures, hence `any[]` here. */
// deno-lint-ignore no-explicit-any
export type MainThreadHandlers = Record<string, (...args: any[]) => unknown>;
export type MainThreadModuleFactory = (ctx: { pushEvent: (message: unknown) => void }) => MainThreadHandlers;
export type MainThreadModuleSource = MainThreadHandlers | MainThreadModuleFactory;

export interface CreateWorkerOpts {
    wasmUrl?: string
    defaults?: boolean
    systemModules?: Record<string, string>
    mainThreadModules?: Record<string, MainThreadModuleSource>
    imports?: Record<string, string>
    integrity?: boolean
    loaders?: string[]
    version?: string | null
}

export interface WorkerHandle {
    integrityActive: boolean
    loadMs: number
    run(src: string, runOpts?: Omit<RunOpts, 'src'>): Promise<ExecResult>
    setPreemptInterval(interval: number): Promise<void>
    pause(): Promise<boolean>
    resume(): Promise<void>
    saveState(): Promise<Uint8Array>
    restoreState(blob: Uint8Array | ArrayBuffer): Promise<ExecResult>
    stateGlobals(): Promise<Record<string, unknown>>
    stateStack(): Promise<unknown[]>
    reset(): Promise<void>
    clearCache(): Promise<void>
    pushEvent(message: unknown): void
    onOutput(handler: (text: string) => void): void
    dispose(): void
}

interface Pending {
    resolve: (v: unknown) => void
    reject: (e: Error) => void
}

// WorkerRequest without the reqId, `send` attaches it.
type DistOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

/* Public entry. `createWorker(opts)` spawns a Web Worker around `engine.ts` and returns a proxy whose methods round-trip via postMessage. */
export async function createWorker(opts?: CreateWorkerOpts): Promise<WorkerHandle> {
    // Chromium blocks `new Worker(crossOriginUrl)` even with `type:'module'`, cross-origin runtimes need the Blob bootstrap below.
    const workerUrl = new URL('./worker/worker.js', import.meta.url);
    const sameOrigin = workerUrl.origin === self.location.origin;
    const worker = sameOrigin
        ? new Worker(workerUrl, { type: 'module' })
        : spawnCrossOriginWorker(workerUrl.href);

    let reqIdCounter = 0;
    const pending = new Map<number, Pending>();
    let outputHandler: ((text: string) => void) | null = null;

    const tell = (msg: WorkerRequest) => worker.postMessage(msg);

    const send = <T = unknown>(payload: DistOmit<WorkerRequest, 'reqId'>): Promise<T> => new Promise((resolve, reject) => {
        const reqId = ++reqIdCounter;
        pending.set(reqId, { resolve: resolve as (v: unknown) => void, reject });
        tell({ ...payload, reqId });
    });

    /* Fire a string into the running script's `receive()` queue. Defined early so main-thread module factories can capture it. */
    const pushEvent = (message: unknown) => tell({ type: 'push-event', message: String(message) });

    /* Resolve each `mainThreadModules[name]` (factory or object) into a flat handler map keyed `module:name`. */
    // deno-lint-ignore no-explicit-any
    const mainThreadHandlers: Record<string, (...args: any[]) => unknown> = {};
    const manifests: MainThreadManifest[] = [];
    for (const [modName, source] of Object.entries(opts?.mainThreadModules || {})) {
        const handlers = typeof source === 'function' ? source({ pushEvent }) : source;
        manifests.push({ name: modName, exports: Object.keys(handlers) });
        for (const [fnName, handler] of Object.entries(handlers)) {
            mainThreadHandlers[`${modName}:${fnName}`] = handler;
        }
    }

    /* Lazy system modules, name to ESM url, imported only when the worker reports the bare name is used. Base defaults sit under user entries, pass `defaults` false to opt out. */
    const systemUrls: Record<string, string> = { ...(opts?.defaults !== false ? DEFAULT_SYSTEM : {}), ...(opts?.systemModules || {}) };
    const loadedSystems = new Map<string, string[]>(); // name to export names, memoized across runs
    const loadSystemModule = async (name: string, manifestUrl?: string): Promise<string[]> => {
        const memo = loadedSystems.get(name);
        if (memo) return memo;
        // Embedder entries win, manifest-declared systems supply their own url.
        const url = systemUrls[name] ?? manifestUrl;
        if (!url) throw new Error(`no system module registered for '${name}'`);
        const mod = await import(url);
        const factory: MainThreadModuleSource = mod[name] ?? mod.default;
        const handlers = typeof factory === 'function' ? factory({ pushEvent }) : factory;
        for (const [fnName, handler] of Object.entries(handlers)) {
            mainThreadHandlers[`${name}:${fnName}`] = handler;
        }
        const exportNames = Object.keys(handlers);
        loadedSystems.set(name, exportNames);
        return exportNames;
    };

    worker.onmessage = async ({ data }: MessageEvent<WorkerMessage>) => {
        switch (data.type) {
            case 'line':
                if (outputHandler) outputHandler(data.text);
                return;
            case 'host-call': {
                const handler = mainThreadHandlers[`${data.module}:${data.name}`];
                if (!handler) {
                    tell({ type: 'host-call-response', reqId: data.reqId, error: `no main-thread handler for '${data.module}.${data.name}'` });
                    return;
                }
                try {
                    const value = await handler(...data.args);
                    tell({ type: 'host-call-response', reqId: data.reqId, value: value as EdgeValue });
                } catch (e) {
                    tell({ type: 'host-call-response', reqId: data.reqId, error: errMsg(e) });
                }
                return;
            }
            case 'load-system': {
                try {
                    const exports = await loadSystemModule(data.name, data.url);
                    tell({ type: 'load-system-response', reqId: data.reqId, exports });
                } catch (e) {
                    tell({ type: 'load-system-response', reqId: data.reqId, error: errMsg(e) });
                }
                return;
            }
            case 'response':
            case 'error': {
                if (data.reqId == null) {
                    // A requestless error is the bootstrap failing, nothing will ever answer.
                    if (data.type === 'error') {
                        for (const cb of pending.values()) cb.reject(new Error(data.message));
                        pending.clear();
                    }
                    return;
                }
                const cb = pending.get(data.reqId);
                if (!cb) return;
                pending.delete(data.reqId);
                if (data.type === 'error') cb.reject(new Error(data.message));
                else cb.resolve(data.result);
                return;
            }
        }
    };

    worker.onerror = (e: ErrorEvent) => {
        const err = new Error(e.message || 'worker error');
        for (const cb of pending.values()) cb.reject(err);
        pending.clear();
    };

    /* Strip mainThreadModules/systemModules before crossing postMessage, not structured-cloneable / loaded on the page. The worker only needs eager manifests and the lazy system names. */
    const { mainThreadModules: _drop, systemModules: _dropSystems, ...workerOpts } = opts || {};
    /* Fold the std .wasm defaults into imports here so the worker engine stays embedder-neutral, pass `defaults` false to opt out. */
    const imports: Record<string, string> = { ...(opts?.defaults !== false ? DEFAULT_IMPORTS : {}), ...(opts?.imports || {}) };
    const ready = await send<{ integrityActive: boolean, loadMs: number }>({
        type: 'load',
        opts: { ...workerOpts, imports, availableSystems: Object.keys(systemUrls) },
        mainThreadManifests: manifests,
    });

    /* Browser bridges fire `CustomEvent("edge-python-event")` on the global, route the detail to the Worker. Gated on `document` to skip Workers / Deno where this listener has no meaning. */
    const onBridgeEvent = (e: Event) => {
        if (typeof (e as CustomEvent).detail === 'string') pushEvent((e as CustomEvent).detail);
    };
    if (typeof document !== 'undefined') addEventListener('edge-python-event', onBridgeEvent);

    return {
        integrityActive: ready.integrityActive,
        loadMs: ready.loadMs,

        run: (src, runOpts = {}) => send<ExecResult>({ type: 'run', src, ...runOpts }),
        /* Preempt every `interval` back-edges, 0 disables. */
        setPreemptInterval: (interval) => send<void>({ type: 'set-preempt-interval', interval }),
        /* Park the program, resolves true when parked. */
        pause: () => send<boolean>({ type: 'pause' }),
        /* Continue a program parked by pause(). */
        resume: () => send<void>({ type: 'resume' }),
        /* Snapshot the paused program, throws when none. */
        saveState: () => send<Uint8Array>({ type: 'save-state' }),
        /* Boot from a blob, resolves like run(). */
        restoreState: (blob) => send<ExecResult>({ type: 'restore-state', blob }),
        stateGlobals: () => send<Record<string, unknown>>({ type: 'state-globals' }),
        stateStack: () => send<unknown[]>({ type: 'state-stack' }),
        reset: () => send<void>({ type: 'reset' }),
        clearCache: () => send<void>({ type: 'clear-cache' }),
        pushEvent,

        onOutput(handler: (text: string) => void) { outputHandler = handler; },

        dispose() {
            if (typeof document !== 'undefined') removeEventListener('edge-python-event', onBridgeEvent);
            tell({ type: 'dispose' });
            worker.terminate();
            for (const cb of pending.values()) cb.reject(new Error('worker disposed'));
            pending.clear();
        },
    };
}

/* Buffers messages until the imported worker.js installs self.onmessage, the first postMessage would be lost otherwise. A source string because tsc rewrites import() in compiled code and the helper would not exist inside the Blob. */
const crossOriginBootstrap = `
const buffered = [];
const enqueue = (event) => buffered.push(event.data);
self.addEventListener('message', enqueue);
import(__workerUrl).then(() => {
    self.removeEventListener('message', enqueue);
    for (const data of buffered) self.dispatchEvent(new MessageEvent('message', { data }));
}, (err) => {
    self.postMessage({ type: 'error', message: 'worker bootstrap failed: ' + ((err && err.message) || err) });
});
`;

/* Blob URL inherits the page's origin, which sidesteps Chromium's cross-origin block. The imported module then loads under CORS (Cloudflare Pages OK by default). */
function spawnCrossOriginWorker(workerUrl: string): Worker {
    const source = `const __workerUrl = ${JSON.stringify(workerUrl)};\n${crossOriginBootstrap}`;
    const blob = new Blob([source], { type: 'application/javascript' });
    const blobUrl = URL.createObjectURL(blob);
    const worker = new Worker(blobUrl, { type: 'module' });
    // Defer revoke a tick, some browsers race it against the module fetch.
    setTimeout(() => URL.revokeObjectURL(blobUrl), 0);
    return worker;
}
