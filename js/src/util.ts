import type { CompilerExports } from './wasm.ts';

export const errMsg = (e: unknown): string => e instanceof Error ? e.message : String(e);

/* A ReferenceError out of a host module names the Web API this runtime lacks. */
export const hostCallError = (module: string, e: unknown): string => {
    const m = e instanceof ReferenceError ? /^(\w+) is not defined$/.exec(e.message) : null;
    return m ? `module '${module}' needs '${m[1]}', missing in this runtime` : errMsg(e);
};

// The RUNTIME error kind in abi/src/lib.rs.
export const ERR_RUNTIME = 2;

export const writeBytes = (exports: CompilerExports, bytes: Uint8Array): number => {
    const ptr = exports.wasm_alloc(Math.max(1, bytes.length));
    new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
};
