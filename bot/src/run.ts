import { MAX_OUTPUT, RUN_MEMORY, RUN_OPS } from './config'

export type Ran = { output: string; error?: string }

type Engine = {
  memory: WebAssembly.Memory
  wasm_alloc(size: number): number
  out_ptr(): number
  out_len(): number
  set_limits(memory: bigint, ops: bigint): void
  set_wall_clock(on: number): void
  run_start(ptr: number, len: number): number
}

// The kind in the status word run_start packs, as src/wasm/exports.rs lays it out.
const KIND_SHIFT = 29
const DONE = 0
const ERROR = 4
const EXIT = 6

// A fresh instance with no grants and nothing to fetch, so a run sees nothing another left behind and reaches nothing outside itself.
export function run(module: WebAssembly.Module, code: string): Ran {
  const printed: string[] = []
  const text = (ptr: number, len: number) => new TextDecoder().decode(new Uint8Array(engine.memory.buffer, ptr, len))

  const engine = new WebAssembly.Instance(module, {
    env: {
      host_print: (ptr: number, len: number) => void printed.push(text(ptr, len)),
      host_now_ns: () => 0n,
      host_send: () => 1,
      host_call_native: () => 1,
      host_fetch_bytes: (_spec: number, _len: number, _hash: number, out: number) => {
        new DataView(engine.memory.buffer).setUint32(out, 0, true)
        return 0
      }
    }
  }).exports as unknown as Engine

  engine.set_limits(BigInt(RUN_MEMORY), BigInt(RUN_OPS))
  engine.set_wall_clock(0)

  const bytes = new TextEncoder().encode(code)
  const at = engine.wasm_alloc(Math.max(1, bytes.length))
  new Uint8Array(engine.memory.buffer, at, bytes.length).set(bytes)

  let status: number
  try {
    status = engine.run_start(at, bytes.length)
  } catch (failure) {
    return { output: printed.join('').slice(0, MAX_OUTPUT), error: `The engine stopped, ${(failure as Error).message}` }
  }

  const kind = (status >>> KIND_SHIFT) & 7
  const output = printed.join('').slice(0, MAX_OUTPUT)

  if (kind === DONE || kind === EXIT) return { output }
  if (kind === ERROR) return { output, error: text(engine.out_ptr(), engine.out_len()).slice(0, MAX_OUTPUT) }
  return { output, error: 'The program waited on input, a message or a clock, and a run here has none of them.' }
}
