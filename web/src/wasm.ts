/* Shape of `compiler.wasm`'s exports, shared by every worker-side module. Optional members are only present on newer builds, callers already guard them with `typeof`/`?.`. */
export interface CompilerExports {
    memory: WebAssembly.Memory
    src_ptr(): number
    out_ptr(): number
    wasm_alloc(size: number): number
    wasm_free(ptr: number, size: number): void
    register_code_module(spec_ptr: number, spec_len: number, src_ptr: number, src_len: number): void
    register_native_module(spec_ptr: number, spec_len: number, names_ptr: number, names_len: number, base_id: number): void
    reset_modules(): void
    set_entry_dir?(len: number): void
    set_input?(ptr: number, len: number): void
    extract_imports?(len: number): number
    repl_eval(len: number): number
    run_start(len: number): number
    run_resume(): number
    run_push_event(ptr: number, len: number): number
    set_host_result(handle: number): number
    set_host_result_by_id(id: number, handle: number): number
    set_host_error_by_id(id: number, kind: number, msg_handle: number): number
    last_yield_deadline_ns(): bigint
    set_preempt_interval?(n: number): void
    save_state(): bigint
    snapshot_ptr(): number
    restore_state(len: number): number
    state_globals(): number
    state_stack(): number
    run(len: number): number
    host_edge_op(op: number, recv: number, name_ptr: number, name_len: number, argv_ptr: number, argc: number, out_handle: number): number
    host_edge_encode(tag: number, ptr: number, len: number): number
    host_edge_decode(h: number, out_tag: number, dst: number, dst_max: number): number
    host_edge_release(h: number): void
    host_edge_throw(kind: number, msg_ptr: number, msg_len: number): void
    host_edge_take_error(out_kind: number, dst: number, dst_max: number): number
}
