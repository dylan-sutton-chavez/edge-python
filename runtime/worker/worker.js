/*
Web Worker entry. Receives postMessage requests from `createWorker`, dispatches to the engine, posts responses.
*/

import * as engine from './engine.js';

const onLine = (text) => self.postMessage({ type: 'line', text });

/* Deferred host calls: post `{type:'host-call', reqId, module, name, args}` to main and await `{type:'host-call-response'}`. */
let nextHostReqId = 0;
const pendingHostCalls = new Map();

engine.setHostCallDelegate((module, name, args) => new Promise((resolve, reject) => {
    const reqId = ++nextHostReqId;
    pendingHostCalls.set(reqId, { resolve, reject });
    self.postMessage({ type: 'host-call', reqId, module, name, args });
}));

/* Lazy host load: post `{type:'load-host', reqId, name, url}` to main and await `{type:'load-host-response'}` with export names. */
let nextLoadHostReqId = 0;
const pendingLoadHost = new Map();

engine.setLoadHostDelegate((name, url) => new Promise((resolve, reject) => {
    const reqId = ++nextLoadHostReqId;
    pendingLoadHost.set(reqId, { resolve, reject });
    self.postMessage({ type: 'load-host', reqId, name, url });
}));

const handlers = {
    load: (data) => engine.load(data.opts, data.mainThreadManifests),
    run: (data) => engine.run({ ...data, onLine }),
    'set-preempt-interval': (data) => engine.setPreemptInterval(data.interval),
    pause: () => engine.pause(),
    resume: () => engine.resume(),
    'save-state': () => engine.saveState(),
    'restore-state': (data) => engine.restoreState({ blob: data.blob, onLine }),
    'state-globals': () => engine.stateGlobals(),
    'state-stack': () => engine.stateStack(),
    reset: () => engine.reset(),
    clearCache: () => engine.clearCache(),
    dispose: () => { engine.dispose(); self.close(); },
    /* Wake a paused `receive()` in the running script; fire-and-forget, no response needed. */
    'push-event': (data) => engine.pushEvent(data.message),
    /* Main thread answered a deferred host call; resolve the waiting delegate Promise. */
    'host-call-response': (data) => {
        const cb = pendingHostCalls.get(data.reqId);
        if (!cb) return;
        pendingHostCalls.delete(data.reqId);
        if (data.error) cb.reject(new Error(data.error));
        else cb.resolve(data.value);
    },
    /* Main thread loaded a lazy host module; resolve with its export names. */
    'load-host-response': (data) => {
        const cb = pendingLoadHost.get(data.reqId);
        if (!cb) return;
        pendingLoadHost.delete(data.reqId);
        if (data.error) cb.reject(new Error(data.error));
        else cb.resolve(data.exports);
    },
};

self.onmessage = async ({ data }) => {
    const handler = handlers[data.type];
    if (!handler) {
        self.postMessage({ type: 'error', reqId: data.reqId, message: `unknown message type: ${data.type}` });
        return;
    }
    try {
        const result = await handler(data);
        /* Fire-and-forget message types skip the response post; only reply when an outer reqId was attached. */
        if (data.reqId != null && data.type !== 'host-call-response' && data.type !== 'load-host-response' && data.type !== 'push-event') {
            self.postMessage({ type: 'response', reqId: data.reqId, result });
        }
    } catch (e) {
        self.postMessage({ type: 'error', reqId: data.reqId, message: e?.message ?? String(e) });
    }
};
