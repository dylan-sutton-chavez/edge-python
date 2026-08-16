import { nativeTable } from './native.ts';
import type { CompilerExports } from './wasm.ts';
import type { Rt, EdgeValue } from './rt.ts';
import { errMsg, writeBytes, ERR_RUNTIME } from './util.ts';

const TD = new TextDecoder();
const TE = new TextEncoder();

export interface DeferredHostCall {
    module: string
    name: string
    args: EdgeValue[]
}

export interface CompilerEnv {
    host_print(ptr: number, len: number): void
    host_call_native(id: number, call_id: number, argv_ptr: number, argc: number, out_ptr: number): number
    host_now_ns(): bigint
    host_fetch_bytes(specPtr: number, specLen: number, hashPtr: number, outLenPtr: number): number
}

export interface MakeCompilerEnvOpts {
    getExports: () => CompilerExports
    onLine: (text: string) => void
    fetchedSources: Map<string, Uint8Array>
    lockfile: Map<string, string>
    integrityActive: boolean
    rt?: Rt
    captureHostCall?: (id: number, call: DeferredHostCall) => void
}

/* The `env.*` imports the compiler declares (host_print, host_call_native, host_fetch_bytes, host_now_ns), wired to closure-captured engine state. */
export function makeCompilerEnv({ getExports, onLine, fetchedSources, lockfile, integrityActive, rt, captureHostCall }: MakeCompilerEnvOpts): CompilerEnv {
    const readStr = (ptr: number, len: number) => TD.decode(new Uint8Array(getExports().memory.buffer, ptr, len));
    const setU32 = (ptr: number, v: number) => new DataView(getExports().memory.buffer).setUint32(ptr, v, true);

    return {
        host_print: (ptr, len) => onLine(readStr(ptr, len)),

        /* wasmpdk stages argv in guest memory, capability calls a JS handler directly. */
        host_call_native: (id, call_id, argv_ptr, argc, out_ptr) => {
            const fn = nativeTable[id];
            if (!fn) {
                stashError(getExports(), `native id ${id} not registered`);
                return 1;
            }

            const exports = getExports();

            if (fn.__edge_kind === 'capability') {
                /* Host appends a trailing kwargs handle (0 = no kwargs), JS capabilities don't model kwargs so drop it. */
                const handles = Array.from(new Uint32Array(exports.memory.buffer, argv_ptr, Math.max(0, argc - 1)));
                /* Marked main-thread, decode args to JS, defer via captureHostCall. Driver wakes us with set_host_result_by_id. */
                if (fn.__edge_main_thread) {
                    if (!captureHostCall || !rt) {
                        stashError(exports, `native '${fn.__edge_module}.${fn.__edge_name}' marked main-thread but no host-call delegate wired`);
                        return 1;
                    }
                    try {
                        const args = handles.map((h) => rt.decodeAny(h));
                        captureHostCall(call_id, { module: fn.__edge_module, name: fn.__edge_name, args });
                        return 2;
                    } catch (e) {
                        stashError(exports, errMsg(e));
                        return 1;
                    }
                }
                try {
                    const resultHandle = fn(handles) as number;
                    new DataView(exports.memory.buffer).setUint32(out_ptr, resultHandle, true);
                    return 0;
                } catch (e) {
                    stashError(exports, errMsg(e));
                    return 1;
                }
            }

            // wasmpdk, stage argv, call, copy back. Views as fns because `fn(...)` can re-enter `wasm_alloc` and detach a cached view.
            const guestView = () => new DataView(fn.__edge_memory.buffer);
            const compView = () => new DataView(exports.memory.buffer);

            const argvLen = Math.max(4, argc * 4);
            const g_argv = fn.__edge_alloc(argvLen);
            const g_out = fn.__edge_alloc(4);
            for (let i = 0; i < argc; i++) {
                guestView().setUint32(g_argv + i * 4, compView().getUint32(argv_ptr + i * 4, true), true);
            }

            const status = fn(g_argv, argc, g_out) as number;
            if (status === 0) {
                compView().setUint32(out_ptr, guestView().getUint32(g_out, true), true);
            }
            // Optional export, pre-__edge_free plugins still leak.
            fn.__edge_free?.(g_argv, argvLen);
            fn.__edge_free?.(g_out, 4);
            return status;
        },

        /* Wall-clock ns as BigInt, wasm marshals to i64 (JS Numbers lose precision past 2^53 ns). */
        host_now_ns: () => BigInt(Date.now()) * 1_000_000n,

        /* Serves cached bytes for packages.json walk-up and `#sha256-...` verification, returns 0 on lockfile drift. */
        host_fetch_bytes: (specPtr, specLen, hashPtr, outLenPtr) => {
            const spec = readStr(specPtr, specLen);
            const bytes = fetchedSources.get(spec);
            if (bytes === undefined) { setU32(outLenPtr, 0); return 0; }

            if (integrityActive && hashPtr !== 0) {
                const knownHex = lockfile.get(spec);
                if (knownHex) {
                    const expected = new Uint8Array(getExports().memory.buffer, hashPtr, 32);
                    const hex = [...expected].map(b => b.toString(16).padStart(2, '0')).join('');
                    if (hex !== knownHex) { setU32(outLenPtr, 0); return 0; }
                }
            }

            const exps = getExports();
            const ptr = exps.wasm_alloc(bytes.length);
            new Uint8Array(exps.memory.buffer, ptr, bytes.length).set(bytes);
            setU32(outLenPtr, bytes.length);
            return ptr;
        },
    };
}

function stashError(exports: CompilerExports, message: string): void {
    const bytes = TE.encode(message);
    const ptr = writeBytes(exports, bytes);
    exports.host_edge_throw(ERR_RUNTIME, ptr, bytes.length);
    exports.wasm_free(ptr, Math.max(1, bytes.length));
}
