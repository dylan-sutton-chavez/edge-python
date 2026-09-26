const TIMEOUT_MS = 10000
const LOAD_MS = 7000

export type Phase = 'runtime' | 'worker' | 'running'

type Worker = {
  run(source: string, options: { baseUrl: string }): Promise<{ out: string; ms: number }>
  onOutput(handler: (chunk: string) => void): void
  dispose(): void
}

let worker: Promise<Worker> | null = null
let ready = false
let sink: ((chunk: string) => void) | null = null
let queue: Promise<unknown> = Promise.resolve()

async function kill() {
  const pending = worker
  worker = null
  ready = false
  sink = null

  try {
    ;(await pending)?.dispose()
  } catch {}
}

// Nothing resolves undeclared, the playground declares every official package like any page would.
// 010100101010 THESE URLS STOPPED RESOLVING WHEN THE CDN DROPPED THE STD, POINT THEM AT THE REGISTRY ONCE EDGE-PYTHON-STD PUBLISHES.
const imports = (cdn: string) => ({
  json: `${cdn}/std/json.wasm`,
  re: `${cdn}/std/re.wasm`,
  math: `${cdn}/std/math.wasm`,
  struct: `${cdn}/std/struct.wasm`,
  test: `${cdn}/std/test.py`,
  dom: `${cdn}/js/builtins/dom/entry.py`,
  storage: `${cdn}/js/builtins/storage/index.js`,
  network: `${cdn}/js/builtins/network/index.js`,
  time: `${cdn}/js/builtins/time/index.js`
})

// The JS host and the compiler come from this environment's CDN, set in wrangler.json.
function spawn(cdn: string, packages: Record<string, string>, onPhase?: (phase: Phase) => void): Promise<Worker> {
  if (worker) return worker

  const load = async () => {
    onPhase?.('runtime')
    const { createWorker } = await import(/* @vite-ignore */ `${cdn}/js/src/index.js`)

    onPhase?.('worker')
    // A package page adds its own package, pinned, and it wins over an official name it shares.
    const spawned: Worker = await createWorker({ wasmUrl: `${cdn}/compiler.wasm`, integrity: true, imports: { ...imports(cdn), ...packages } })
    spawned.onOutput((chunk) => sink?.(chunk))
    ready = true

    return spawned
  }

  const silence = new Promise<never>((_, reject) => {
    setTimeout(() => reject(new Error(`No response after ${LOAD_MS / 1000}s`)), LOAD_MS)
  })

  worker = Promise.race([load(), silence]).catch((error) => {
    console.error(error)
    throw new Error("Couldn't load the runtime. Check your connection and try again.")
  })

  return worker
}

export async function run(
  source: string,
  cdn: string,
  onChunk: (chunk: string) => void,
  onPhase?: (phase: Phase) => void,
  packages: Record<string, string> = {}
): Promise<{ error: string; ms: number }> {
  const exec = async () => {
    const active = await spawn(cdn, packages, ready ? undefined : onPhase)
    onPhase?.('running')
    sink = onChunk

    let timer: ReturnType<typeof setTimeout> | undefined

    try {
      const running = active.run(source, { baseUrl: location.href })
      running.catch(() => {})

      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`Run exceeded ${TIMEOUT_MS / 1000}s, worker terminated`)),
          TIMEOUT_MS
        )
      })

      const { out, ms } = await Promise.race([running, timeout])
      return { error: out || '', ms }
    } catch (error) {
      await kill()
      throw error
    } finally {
      clearTimeout(timer)
      sink = null
    }
  }

  const result = queue.then(exec, exec)
  queue = result.catch(() => {})

  return result
}
