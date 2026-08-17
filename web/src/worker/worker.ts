import * as engine from './engine.ts';
import type { EdgeValue } from '../rt.ts';
import { errMsg } from '../util.ts';
import type { WorkerRequest, WorkerMessage } from '../protocol.ts';

const post = (msg: WorkerMessage) => self.postMessage(msg);
const onLine = (text: string) => post({ type: 'line', text });

/* A deferred request/response round-trip to the main thread, `call` posts and parks, `settle` resolves the parked Promise. */
function makeRpc<Args extends unknown[], Res>(send: (reqId: number, ...args: Args) => void) {
    let nextId = 0;
    const pending = new Map<number, { resolve: (v: Res) => void, reject: (e: Error) => void }>();
    return {
        call: (...args: Args) => new Promise<Res>((resolve, reject) => {
            const reqId = ++nextId;
            pending.set(reqId, { resolve, reject });
            send(reqId, ...args);
        }),
        settle: (resp: { reqId: number } & ({ error: string } | { value: Res })) => {
            const cb = pending.get(resp.reqId);
            if (!cb) return;
            pending.delete(resp.reqId);
            if ('error' in resp) cb.reject(new Error(resp.error));
            else cb.resolve(resp.value);
        },
    };
}

const hostCalls = makeRpc<[string, string, EdgeValue[]], EdgeValue>(
    (reqId, module, name, args) => post({ type: 'host-call', reqId, module, name, args }));
engine.setHostCallDelegate(hostCalls.call);

const systemLoads = makeRpc<[string, string | undefined], string[]>(
    (reqId, name, url) => post({ type: 'load-system', reqId, name, url }));
engine.setLoadSystemDelegate(systemLoads.call);

/* Fire-and-forget messages return this instead of a result, no 'response' is posted for them. */
const NO_REPLY: unique symbol = Symbol('no-reply');

function dispatch(req: WorkerRequest): unknown {
    switch (req.type) {
        case 'load': return engine.load(req.opts, req.mainThreadManifests);
        case 'run': return engine.run({ src: req.src, repl: req.repl, entryDir: req.entryDir, baseUrl: req.baseUrl, incremental: req.incremental, input: req.input }, onLine);
        case 'set-preempt-interval': return engine.setPreemptInterval(req.interval);
        case 'pause': return engine.pause();
        case 'resume': return engine.resume();
        case 'save-state': return engine.saveState();
        case 'restore-state': return engine.restoreState({ blob: req.blob, onLine });
        case 'state-globals': return engine.stateGlobals();
        case 'state-stack': return engine.stateStack();
        case 'reset': return engine.reset();
        case 'clear-cache': return engine.clearCache();
        case 'dispose': engine.dispose(); self.close(); return NO_REPLY;
        // Wake a paused `receive()` in the running script.
        case 'push-event': engine.pushEvent(req.message); return NO_REPLY;
        // Main thread answered a deferred call, resolve the parked delegate Promise.
        case 'host-call-response': hostCalls.settle(req); return NO_REPLY;
        case 'load-system-response':
            systemLoads.settle('error' in req ? req : { reqId: req.reqId, value: req.exports });
            return NO_REPLY;
        default: {
            // Unreachable per the types, reached only on main/worker version drift.
            const _exhaustive: never = req;
            throw new Error(`unknown message type: ${JSON.stringify(_exhaustive)}`);
        }
    }
}

/* Web Worker entry, receives postMessage requests from `createWorker`, dispatches to the engine, posts responses. */
self.onmessage = async ({ data }: MessageEvent<WorkerRequest>) => {
    try {
        const result = await dispatch(data);
        if (result === NO_REPLY) return;
        post({ type: 'response', reqId: data.reqId, result });
    } catch (e) {
        post({ type: 'error', reqId: data.reqId, message: errMsg(e) });
    }
};
