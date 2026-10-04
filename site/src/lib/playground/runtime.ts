import type { TraceEvent } from '../../../../js/src/system/trace'

const TIMEOUT_MS = 10000
const LOAD_MS = 7000

export type Phase = 'lock' | 'runtime' | 'worker' | 'running'

// The edge.json an example runs under, its imports and its grants reach the room.
export type Manifest = { imports?: Record<string, string>; permissions?: Record<string, string[]> }

// Three numbers name a release, which the playground locks the way `edge lock` does.
const RELEASE = /^\d+\.\d+\.\d+$/

/* Each declared release as the url and digest the registry pins it to, so the room opens on pinned bytes. */
async function lock(imports: Record<string, string>): Promise<Record<string, string>> {
  const pinned = await Promise.all(Object.entries(imports).map(async ([name, target]) => {
    if (!RELEASE.test(target)) return [name, target]

    const response = await fetch(`/api/resolve/package/${encodeURIComponent(name)}?v=${target}&lock=1`, { signal: AbortSignal.timeout(LOAD_MS) })
    const answer = (await response.json().catch(() => ({}))) as { error?: string; url?: string; digest?: string }
    if (response.status === 404) throw new Error(`'${name}' has no version ${target}`)
    if (!response.ok) throw new Error(answer.error ?? `asking the registry about '${name}' answered ${response.status}`)

    return [name, `${answer.url}#sha256-${answer.digest}`]
  }))

  return Object.fromEntries(pinned)
}

type Worker = {
  run(source: string, opts?: { entry?: string }): Promise<{ out: string; ms: number }>
  onOutput(handler: (chunk: string) => void): void
  // Missing from a host built before runs reported what they reached.
  onTrace?(handler: (event: TraceEvent) => void): void
  dispose(): void
}

// One room per manifest, since a room's grants and the hosts it reaches are fixed once it opens.
const rooms = new Map<string, Promise<Worker>>()
const ready = new Set<string>()
let sink: ((chunk: string) => void) | null = null
let tracer: ((event: TraceEvent) => void) | null = null
let queue: Promise<unknown> = Promise.resolve()

async function kill(key: string) {
  const pending = rooms.get(key)
  rooms.delete(key)
  ready.delete(key)
  sink = null
  tracer = null

  try {
    ;(await pending)?.dispose()
  } catch {}
}

// The JS host and the compiler come from this environment's CDN, set in wrangler.json.
function spawn(cdn: string, key: string, manifest: Manifest, onPhase?: (phase: Phase) => void): Promise<Worker> {
  const open = rooms.get(key)
  if (open) return open

  const load = async (imports: Record<string, string>) => {
    onPhase?.('runtime')
    const { createWorker } = await import(/* @vite-ignore */ `${cdn}/js/src/index.js`)

    onPhase?.('worker')
    const spawned: Worker = await createWorker({ wasmUrl: `${cdn}/compiler.wasm`, imports, permissions: manifest.permissions ?? {}, trace: true })
    spawned.onOutput((chunk) => sink?.(chunk))
    spawned.onTrace?.((event) => tracer?.(event))
    ready.add(key)

    return spawned
  }

  const declared = manifest.imports ?? {}
  if (Object.values(declared).some((target) => RELEASE.test(target))) onPhase?.('lock')

  // A failed lock says why, as edge lock would, only silence reads as a connection problem.
  const opened = lock(declared).then((imports) => {
    const silence = new Promise<never>((_, reject) => {
      setTimeout(() => reject(new Error(`No response after ${LOAD_MS / 1000}s`)), LOAD_MS)
    })

    return Promise.race([load(imports), silence]).catch((error) => {
      console.error(error)
      throw new Error("Couldn't load the runtime. Check your connection and try again.")
    })
  })

  // A room that never opened is forgotten, so the next run tries again.
  const worker = opened.catch((error) => {
    rooms.delete(key)
    throw error
  })

  rooms.set(key, worker)
  return worker
}

export async function run(
  source: string,
  cdn: string,
  manifest: Manifest,
  onChunk: (chunk: string) => void,
  onPhase?: (phase: Phase) => void,
  onTrace?: (event: TraceEvent) => void
): Promise<{ error: string; ms: number; traced: boolean }> {
  // Two examples whose manifests read the same share one room.
  const key = JSON.stringify(manifest)

  const exec = async () => {
    const active = await spawn(cdn, key, manifest, ready.has(key) ? undefined : onPhase)
    onPhase?.('running')
    sink = onChunk
    tracer = onTrace ?? null

    let timer: ReturnType<typeof setTimeout> | undefined

    try {
      const running = active.run(source, { entry: 'main.py' })
      running.catch(() => {})

      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`Run exceeded ${TIMEOUT_MS / 1000}s, worker terminated`)),
          TIMEOUT_MS
        )
      })

      const { out, ms } = await Promise.race([running, timeout])
      return { error: out || '', ms, traced: typeof active.onTrace === 'function' }
    } catch (error) {
      await kill(key)
      throw error
    } finally {
      clearTimeout(timer)
      sink = null
      tracer = null
    }
  }

  const result = queue.then(exec, exec)
  queue = result.catch(() => {})

  return result
}
