/* Shapes crossing the main-thread/worker postMessage boundary. Plain data only, no DOM or WebWorker globals, so both the `dom` and `webworker` lib scopes can import it without mixing libs in one program. */

import type { EdgeValue } from './rt.ts';

export interface LoadOpts {
    wasmUrl?: string
    integrity?: boolean
    loaders?: string[]
    imports?: Record<string, string> | null
    version?: string | null
    availableSystems?: string[]
}

export interface MainThreadManifest {
    name: string
    exports: string[]
}

export interface RunOpts {
    src: string
    repl?: boolean
    entryDir?: string
    baseUrl?: string | null
    incremental?: boolean
}

export interface ExecResult {
    out: string
    ms: number
    exitCode?: number
}

export type HostCallResponse =
    | { type: 'host-call-response', reqId: number, value: EdgeValue }
    | { type: 'host-call-response', reqId: number, error: string };

export type LoadSystemResponse =
    | { type: 'load-system-response', reqId: number, exports: string[] }
    | { type: 'load-system-response', reqId: number, error: string };

/* Requests main to worker. `reqId` correlates each 'response'/'error' answer, fire-and-forget types omit it. */
export type WorkerRequest =
    | { type: 'load', reqId: number, opts: LoadOpts, mainThreadManifests: MainThreadManifest[] }
    | ({ type: 'run', reqId: number } & RunOpts)
    | { type: 'set-preempt-interval', reqId: number, interval: number }
    | { type: 'pause', reqId: number }
    | { type: 'resume', reqId: number }
    | { type: 'save-state', reqId: number }
    | { type: 'restore-state', reqId: number, blob: Uint8Array | ArrayBuffer }
    | { type: 'state-globals', reqId: number }
    | { type: 'state-stack', reqId: number }
    | { type: 'reset', reqId: number }
    | { type: 'clear-cache', reqId: number }
    | { type: 'push-event', reqId?: number, message: string }
    | { type: 'dispose', reqId?: number }
    | HostCallResponse
    | LoadSystemResponse;

/* Pushes worker to main. 'response' answers a request's reqId, the rest are unsolicited. */
export type WorkerMessage =
    | { type: 'line', text: string }
    | { type: 'host-call', reqId: number, module: string, name: string, args: EdgeValue[] }
    | { type: 'load-system', reqId: number, name: string, url?: string }
    | { type: 'response', reqId?: number, result: unknown }
    | { type: 'error', reqId?: number, message: string };
