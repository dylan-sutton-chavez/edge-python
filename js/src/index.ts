import type { Limits, RunOpts, ExecResult, WorkerRequest, WorkerMessage } from './protocol.ts';
import type { Permissions } from './system/grants.ts';
import type { TraceEvent } from './system/trace.ts';
import { roomScript } from './room.ts';

export type { TraceEvent };

export interface CreateWorkerOpts {
    // The program's directory, the page reads its files and its edge.json for the room.
    baseUrl?: string
    wasmUrl?: string
    imports?: Record<string, string>
    permissions?: Permissions
    secrets?: Record<string, string>
    trace?: boolean
    limits?: Limits | null
}

export interface WorkerHandle {
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
    onTrace(handler: (event: TraceEvent) => void): void
    dispose(): void
}

interface Pending {
    resolve: (v: unknown) => void
    reject: (e: Error) => void
}

// WorkerRequest without the reqId, `send` attaches it.
type DistOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

/* Public entry. `createWorker(opts)` runs one program in a room of its own and returns a proxy whose methods round-trip via postMessage. */
export async function createWorker(opts: CreateWorkerOpts = {}): Promise<WorkerHandle> {
    if (typeof document === 'undefined') throw new Error('createWorker needs a page, missing in this runtime');
    const base = opts.baseUrl ? new URL('./', opts.baseUrl).href : null;
    // The compiler sits beside the host wherever it ships, the CDN, a dist and the CLI's server.
    const wasmUrl = opts.wasmUrl ?? new URL('../../compiler.wasm', import.meta.url).href;
    // The page fetches the engine and hands it over, so the room loads no code of its own.
    const [source, wasm, root] = await Promise.all([
        download(new URL('./worker/bundle.js', import.meta.url).href).then((r) => r.text()),
        download(wasmUrl).then((r) => r.arrayBuffer()),
        rootOf(opts, base),
    ]);
    const { port, close } = await openRoom(source, await roomPolicy(root));

    let reqIdCounter = 0;
    const pending = new Map<number, Pending>();
    let outputHandler: ((text: string) => void) | null = null;
    let traceHandler: ((event: TraceEvent) => void) | null = null;

    const tell = (msg: WorkerRequest) => port.postMessage(msg);

    const send = <T = unknown>(payload: DistOmit<WorkerRequest, 'reqId'>): Promise<T> => new Promise((resolve, reject) => {
        const reqId = ++reqIdCounter;
        pending.set(reqId, { resolve: resolve as (v: unknown) => void, reject });
        tell({ ...payload, reqId });
    });

    /* Fire a string into the running script's `receive()` queue. */
    const pushEvent = (message: unknown) => tell({ type: 'push-event', message: String(message) });

    /* Reads a file of the program for the room, only inside its directory and never with the page's cookies. */
    const read = async (id: number, url: string): Promise<void> => {
        const reply = { type: 'file' as const, id, status: 0, contentType: '', body: null as ArrayBuffer | null };
        try {
            if (base && new URL(url).href.startsWith(base)) {
                const res = await fetch(url, { credentials: 'omit' });
                reply.status = res.ok ? 200 : res.status;
                reply.contentType = res.headers.get('content-type') ?? '';
                reply.body = res.ok ? await res.arrayBuffer() : null;
            }
        } catch { /* status 0 reads as a failed fetch in the room */ }
        tell(reply);
    };

    port.onmessage = ({ data }: MessageEvent<WorkerMessage>) => {
        switch (data.type) {
            case 'line':
                if (outputHandler) outputHandler(data.text);
                return;
            case 'trace':
                traceHandler?.(data.event);
                return;
            case 'read':
                void read(data.id, data.url);
                return;
            case 'response':
            case 'error': {
                if (data.reqId == null) {
                    // A requestless error is the worker failing, nothing will ever answer.
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

    // A room whose engine fails to start leaves no frame behind.
    const ready = await send<{ loadMs: number }>({ type: 'load', opts: { ...opts, baseUrl: base, wasm } })
        .catch((e: unknown) => { close(); throw e; });

    return {
        loadMs: ready.loadMs,

        // A worker runs the one program its baseUrl names, so a run cannot point elsewhere.
        run: (src, runOpts = {}) => 'baseUrl' in runOpts
            ? Promise.reject(new Error('baseUrl belongs to createWorker, a worker runs one program'))
            : send<ExecResult>({ type: 'run', src, ...runOpts }),
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
        /* What each run reaches, reported only by a worker created with `trace: true`. */
        onTrace(handler: (event: TraceEvent) => void) { traceHandler = handler; },

        dispose() {
            tell({ type: 'dispose' });
            close();
            for (const cb of pending.values()) cb.reject(new Error('worker disposed'));
            pending.clear();
        },
    };
}

/* A file of the engine, fetched by the page so the room never loads code. */
async function download(url: string): Promise<Response> {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`fetch failed for '${url}' (${res.status})`);
    return res;
}

/* The specs and grants of the program's root manifest, the page's own or the edge.json at its base. */
async function rootOf(opts: CreateWorkerOpts, base: string | null): Promise<{ specs: string[], permissions: Permissions }> {
    if (opts.imports || opts.permissions || !base) return { specs: Object.values(opts.imports ?? {}), permissions: opts.permissions ?? {} };
    const json = (name: string): Promise<Record<string, unknown>> =>
        fetch(new URL(name, base), { credentials: 'omit' }).then((r) => (r.ok ? r.json() : {})).catch(() => ({}));
    const manifest = await json('edge.json');
    const specs = Object.values((manifest['imports'] ?? {}) as Record<string, unknown>).map(String);
    // The lock beside the manifest names where each release lives, so the room may reach it.
    const locked = Object.values(await json('edge.lock')).map((entry) => String((entry as { url?: unknown } | null)?.url ?? ''));
    return { specs: [...specs, ...locked], permissions: (manifest['permissions'] ?? {}) as Permissions };
}

// A policy names only plain origins and hosts, anything else stays out of reach.
const PLAIN_ORIGIN = /^https?:\/\/[a-z0-9.-]+(:\d+)?$/;
const PLAIN_HOST = /^[a-z0-9.-]+$/;

/* The origin of an absolute url, when a policy can name it as written. */
function originOf(spec: string): string[] {
    try {
        const { origin } = new URL(spec);
        return PLAIN_ORIGIN.test(origin) ? [origin] : [];
    } catch {
        return [];
    }
}

/* The room runs its own script and the engine, and connects only to its imports and its grants. */
async function roomPolicy({ specs, permissions }: { specs: string[], permissions: Permissions }): Promise<string> {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(roomScript)));
    const hosts = Object.values(permissions).flat().flatMap((entry) => {
        // A path prefix bounds the reach inside net, the policy only ever names a whole host.
        const name = String(entry).startsWith('net:') ? String(entry).slice(4).split('/')[0]! : '';
        return PLAIN_HOST.test(name) ? ['https', 'http', 'wss', 'ws'].map((scheme) => `${scheme}://${name}:*`) : [];
    });
    const reach = [...new Set([...specs.flatMap(originOf), ...hosts])];
    return [
        "default-src 'none'",
        `script-src 'sha256-${btoa(String.fromCharCode(...digest))}' 'wasm-unsafe-eval'`,
        'worker-src blob:',
        `connect-src ${reach.length > 0 ? reach.join(' ') : "'none'"}`,
    ].join('; ');
}

/* A sandboxed frame with an opaque origin, no cookie, storage or DOM of the page reaches it. */
async function openRoom(source: string, policy: string): Promise<{ port: MessagePort, close: () => void }> {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.hidden = true;
    frame.srcdoc = `<!DOCTYPE html><meta http-equiv="Content-Security-Policy" content="${policy}"><script>${roomScript}</script>`;
    const loaded = new Promise((resolve) => { frame.onload = resolve; });
    document.body.append(frame);
    await loaded;
    const { port1, port2 } = new MessageChannel();
    frame.contentWindow?.postMessage(source, '*', [port2]);
    return { port: port1, close: () => { port1.close(); frame.remove(); } };
}
