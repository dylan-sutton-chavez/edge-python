import type { CompilerExports } from './wasm.ts';

export const errMsg = (e: unknown): string => e instanceof Error ? e.message : String(e);

// The RUNTIME error kind in abi/src/lib.rs.
export const ERR_RUNTIME = 2;

export const writeBytes = (exports: CompilerExports, bytes: Uint8Array): number => {
    const ptr = exports.wasm_alloc(Math.max(1, bytes.length));
    new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
};
