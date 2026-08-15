import type { CompilerExports } from './wasm.ts';

/* A registered native fn, tagged with dispatch metadata by the loader that produced it. */
export interface NativeFn {
    (...args: unknown[]): unknown
    __edge_kind?: 'wasmpdk' | 'capability'
    __edge_name?: string
    __edge_module?: string
    __edge_main_thread?: boolean
    __edge_alloc?: (size: number) => number
    __edge_free?: (ptr: number, len: number) => void
    __edge_memory?: WebAssembly.Memory
}

export interface NativeModuleResult {
    kind: string
    names: string[]
    fns: NativeFn[]
}

export interface NativeLoader {
    match(module: WebAssembly.Module): boolean
    load(module: WebAssembly.Module, ctx: NativeLoadCtx): Promise<NativeModuleResult>
}

export interface NativeLoadCtx {
    loaders: NativeLoader[]
    compilerExports: CompilerExports
}

/* `nativeTable` is indexed by `baseId` from `register_native_module`, entries are wasmpdk fns or JS handlers, dispatched by `host_call_native`. */
export const nativeTable: NativeFn[] = [];

export function resetNativeTable(): void {
    nativeTable.length = 0;
}

/* Build the 6 `env.edge_*` imports for wasm-pdk plugins, bridging guest and compiler memory. */
export function makeGuestEnv(compilerExports: CompilerExports) {
    const compMem = () => new Uint8Array(compilerExports.memory.buffer);
    const compView = () => new DataView(compilerExports.memory.buffer);

    return (guestExports: { memory: WebAssembly.Memory }) => {
        const gMem = () => new Uint8Array(guestExports.memory.buffer);
        const gView = () => new DataView(guestExports.memory.buffer);

        const stage = (ptr: number, len: number): number => {
            const c = compilerExports.wasm_alloc(Math.max(1, len));
            if (len) compMem().set(gMem().subarray(ptr, ptr + len), c);
            return c;
        };
        const unstage = (c: number, len: number): void => compilerExports.wasm_free(c, Math.max(1, len));

        return {
            edge_op: (op: number, recv: number, name_ptr: number, name_len: number, argv_ptr: number, argc: number, out: number): number => {
                const cName = stage(name_ptr, name_len);
                const argvLen = Math.max(4, argc * 4);
                const cArgv = compilerExports.wasm_alloc(argvLen);
                const cOut = compilerExports.wasm_alloc(4);
                for (let i = 0; i < argc; i++) {
                    compView().setUint32(cArgv + i * 4, gView().getUint32(argv_ptr + i * 4, true), true);
                }
                const ret = compilerExports.host_edge_op(op, recv, cName, name_len, cArgv, argc, cOut);
                if (ret === 0 && out) gView().setUint32(out, compView().getUint32(cOut, true), true);
                unstage(cName, name_len);
                compilerExports.wasm_free(cArgv, argvLen);
                compilerExports.wasm_free(cOut, 4);
                return ret;
            },

            edge_encode: (tag: number, ptr: number, len: number): number => {
                const c = stage(ptr, len);
                const h = compilerExports.host_edge_encode(tag, c, len);
                unstage(c, len);
                return h;
            },

            edge_decode: (h: number, out_tag: number, dst: number, dst_max: number): number => {
                const bufLen = Math.max(1, dst_max);
                const cTag = compilerExports.wasm_alloc(4);
                const cBuf = compilerExports.wasm_alloc(bufLen);
                const ret = compilerExports.host_edge_decode(h, cTag, cBuf, dst_max);
                gView().setUint32(out_tag, compView().getUint32(cTag, true), true);
                if (ret > 0) gMem().set(compMem().subarray(cBuf, cBuf + ret), dst);
                compilerExports.wasm_free(cTag, 4);
                compilerExports.wasm_free(cBuf, bufLen);
                return ret;
            },

            edge_release: (h: number): void => compilerExports.host_edge_release(h),

            edge_throw: (kind: number, msg_ptr: number, msg_len: number): void => {
                const c = stage(msg_ptr, msg_len);
                compilerExports.host_edge_throw(kind, c, msg_len);
                unstage(c, msg_len);
            },

            edge_take_error: (out_kind: number, dst: number, dst_max: number): number => {
                const bufLen = Math.max(1, dst_max);
                const cKind = compilerExports.wasm_alloc(4);
                const cBuf = compilerExports.wasm_alloc(bufLen);
                const ret = compilerExports.host_edge_take_error(cKind, cBuf, dst_max);
                if (ret >= 0) {
                    gView().setUint32(out_kind, compView().getUint32(cKind, true), true);
                    if (ret > 0) gMem().set(compMem().subarray(cBuf, cBuf + ret), dst);
                }
                compilerExports.wasm_free(cKind, 4);
                compilerExports.wasm_free(cBuf, bufLen);
                return ret;
            },
        };
    };
}

/* Built-in Path A fallback, instantiate guest, walk exports, annotate each fn with its guest's `__edge_alloc` + `__edge_memory`. */
async function builtinWasmPdkLoader(module: WebAssembly.Module, ctx: NativeLoadCtx): Promise<NativeModuleResult> {
    const envFactory = makeGuestEnv(ctx.compilerExports);
    // Forward reference, the getter captures `instance` lazily. It's only read when env functions fire during VM execution, by which point `instance` is bound.
    const env = envFactory({ get memory() { return instance.exports.memory as WebAssembly.Memory; } });
    // WebAssembly.instantiate(Module, ...) returns the Instance directly, not {module, instance}.
    const instance = await WebAssembly.instantiate(module, { env });

    if (typeof instance.exports.__edge_alloc !== 'function') {
        throw new Error(
            `native module missing '__edge_alloc(size: u32) -> *mut u8';` +
            ` see /reference/abi for the contract`
        );
    }

    const names: string[] = [];
    const fns: NativeFn[] = [];
    for (const [k, value] of Object.entries(instance.exports)) {
        if (k === 'memory' || typeof value !== 'function') continue;
        // Keep convention exports (__fn_/__class_/__const_), drop ABI internals like __edge_alloc.
        if (k.startsWith('__') && !k.startsWith('__class_') && !k.startsWith('__const_') && !k.startsWith('__fn_')) continue;
        const v = value as NativeFn;
        names.push(k);
        v.__edge_alloc = instance.exports.__edge_alloc as (size: number) => number;
        // Optional on older plugins, callers use `?.`.
        v.__edge_free = instance.exports.__edge_free as ((ptr: number, len: number) => void) | undefined;
        v.__edge_memory = instance.exports.memory as WebAssembly.Memory;
        v.__edge_kind = 'wasmpdk';
        fns.push(v);
    }
    return { kind: 'wasmpdk', names, fns };
}

/* Try custom loaders first, built-in Path A is the implicit fallback. */
export async function loadNativeModule(_spec: string, bytes: Uint8Array, ctx: NativeLoadCtx): Promise<NativeModuleResult> {
    const module = await WebAssembly.compile(bytes as BufferSource);

    for (const loader of ctx.loaders) {
        if (loader.match(module)) {
            const result = await loader.load(module, ctx);
            // Tag each fn with its dispatch kind so host_call_native picks the right path.
            for (const fn of result.fns) fn.__edge_kind = result.kind as 'wasmpdk' | 'capability';
            annotateNames(result);
            return result;
        }
    }

    const result = await builtinWasmPdkLoader(module, ctx);
    annotateNames(result);
    return result;
}

/* Pair each fn with its declared name so deferred dispatch can route by name on the main thread. */
function annotateNames({ names, fns }: NativeModuleResult): void {
    for (let i = 0; i < fns.length; i++) fns[i].__edge_name = names[i];
}
