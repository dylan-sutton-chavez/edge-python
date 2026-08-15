import * as engine from './engine.ts';
import type { EdgeValue } from '../rt.ts';
import type { LoadOpts, MainThreadManifest, RunOpts } from '../protocol.ts';

const onLine = (text: string) => self.postMessage({ type: 'line', text });

/* Deferred host calls post `{type:'host-call', reqId, module, name, args}` to main and await `{type:'host-call-response'}`. */
let nextHostReqId = 0;
const pendingHostCalls = new Map<number, { resolve: (value: EdgeValue | PromiseLike<EdgeValue>) => void, reject: (e: Error) => void }>();

engine.setHostCallDelegate((module, name, args) => new Promise((resolve, reject) => {
    const reqId = ++nextHostReqId;
    pendingHostCalls.set(reqId, { resolve, reject });
    self.postMessage({ type: 'host-call', reqId, module, name, args });
}));

/* Lazy system loads post `{type:'load-system', reqId, name, url}` to main and await `{type:'load-system-response'}` with export names. */
let nextLoadSystemReqId = 0;
const pendingLoadSystem = new Map<number, { resolve: (names: string[]) => void, reject: (e: Error) => void }>();

engine.setLoadSystemDelegate((name, url) => new Promise((resolve, reject) => {
    const reqId = ++nextLoadSystemReqId;
    pendingLoadSystem.set(reqId, { resolve, reject });
    self.postMessage({ type: 'load-system', reqId, name, url });
}));

// Requests are the postMessage protocol from `createWorker`/`index.ts`, deliberately untyped past `type`/`reqId`, each handler picks its own fields off `data`.
type WorkerRequest = { type: string, reqId?: number } & Record<string, unknown>;

const handlers: Record<string, (data: WorkerRequest) => unknown> = {
    load: (data) => engine.load(data.opts as LoadOpts, data.mainThreadManifests as MainThreadManifest[]),
    run: (data) => engine.run({ ...data, onLine } as unknown as RunOpts),
    'set-preempt-interval': (data) => engine.setPreemptInterval(data.interval as number),
    pause: () => engine.pause(),
    resume: () => engine.resume(),
    'save-state': () => engine.saveState(),
    'restore-state': (data) => engine.restoreState({ blob: data.blob as Uint8Array | ArrayBuffer, onLine }),
    'state-globals': () => engine.stateGlobals(),
    'state-stack': () => engine.stateStack(),
    reset: () => engine.reset(),
    clearCache: () => engine.clearCache(),
    dispose: () => { engine.dispose(); self.close(); },
    /* Wake a paused `receive()` in the running script, fire-and-forget, no response needed. */
    'push-event': (data) => engine.pushEvent(data.message),
    /* Main thread answered a deferred host call, resolve the waiting delegate Promise. */
    'host-call-response': (data) => {
        const reqId = data.reqId as number;
        const cb = pendingHostCalls.get(reqId);
        if (!cb) return;
        pendingHostCalls.delete(reqId);
        if (data.error) cb.reject(new Error(data.error as string));
        else cb.resolve(data.value as EdgeValue);
    },
    /* Main thread loaded a lazy system module, resolve with its export names. */
    'load-system-response': (data) => {
        const reqId = data.reqId as number;
        const cb = pendingLoadSystem.get(reqId);
        if (!cb) return;
        pendingLoadSystem.delete(reqId);
        if (data.error) cb.reject(new Error(data.error as string));
        else cb.resolve(data.exports as string[]);
    },
};

/* Web Worker entry, receives postMessage requests from `createWorker`, dispatches to the engine, posts responses. */
self.onmessage = async ({ data }: MessageEvent<WorkerRequest>) => {
    const handler = handlers[data.type];
    if (!handler) {
        self.postMessage({ type: 'error', reqId: data.reqId, message: `unknown message type: ${data.type}` });
        return;
    }
    try {
        const result = await handler(data);
        /* Fire-and-forget message types skip the response post, only reply when an outer reqId was attached. */
        if (data.reqId != null && data.type !== 'host-call-response' && data.type !== 'load-system-response' && data.type !== 'push-event') {
            self.postMessage({ type: 'response', reqId: data.reqId, result });
        }
    } catch (e) {
        self.postMessage({ type: 'error', reqId: data.reqId, message: e instanceof Error ? e.message : String(e) });
    }
};
