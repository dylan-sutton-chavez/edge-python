// Shared Edge Python runtime for doc playgrounds. One Web Worker per page (lazy init): loads CDN runtime + compiler.wasm, streams stdout to the active block.
const WASM_URL = 'https://cdn.edgepython.com/compiler.wasm'
const RUNTIME_URL = 'https://cdn.edgepython.com/web/src/index.js'

const RUN_TIMEOUT_MS = 10000 // hard per-run wall-clock cap so a hung snippet can't wedge the page queue

let workerPromise = null
let workerReady = false // flips true once the worker+wasm are loaded; gates the cold-start phases
let activeSink = null // raw-stdout-chunk handler of the block currently running
let runChain = Promise.resolve() // serializes runs: one shared worker + one global activeSink can't host two blocks at once

// Terminate the shared worker and reset state so the next run respawns cold.
async function killWorker() {
	const wp = workerPromise
	workerPromise = null
	workerReady = false
	activeSink = null
	try { (await wp)?.dispose() } catch {}
}

// `onPhase` only matters for the first (cold) call: 'runtime' (downloading the ESM) then 'worker' (spawn + wasm fetch/instantiate).
function getWorker(onPhase) {
	if (workerPromise) return workerPromise
	workerPromise = (async () => {
		onPhase?.('runtime')
		// webpackIgnore keeps Next from bundling the cross-origin ESM; it loads in the browser at runtime.
		const { createWorker } = await import(/* webpackIgnore: true */ RUNTIME_URL)
		onPhase?.('worker')
		const worker = await createWorker({ wasmUrl: WASM_URL, integrity: true })
		worker.onOutput((chunk) => { if (activeSink) activeSink(chunk) })
		workerReady = true
		return worker
	})()
	return workerPromise
}

/* Run `src`, streaming each raw stdout chunk to `onChunk`. `onPhase` reports 'runtime'/'worker' (cold start only) then 'running'. Resolves with the error text (empty on success) and elapsed ms. */
export async function run(src, onChunk, onPhase) {
	const exec = async () => {
		// Only the run that triggers the cold start drives the load phases; warm runs go straight to 'running'.
		const worker = await getWorker(workerReady ? undefined : onPhase)
		onPhase?.('running')
		activeSink = onChunk
		let timer
		try {
			// Race against a hard timeout; terminate() kills the worker even mid-infinite-loop.
			const runP = worker.run(src, { baseUrl: location.href })
			runP.catch(() => {}) // loser of the race; killWorker rejects it with nobody listening
			const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Run exceeded ${RUN_TIMEOUT_MS / 1000}s — worker terminated`)), RUN_TIMEOUT_MS) })
			const { out, ms } = await Promise.race([runP, timeout])
			return { error: out || '', ms }
		} catch (e) {
			// Timeout or worker death: respawn so the queue isn't wedged forever.
			await killWorker()
			throw e
		} finally {
			clearTimeout(timer)
			activeSink = null
		}
	}
	// Queue behind any in-flight run (success or failure) so a second block never overwrites the first's activeSink mid-stream.
	const result = runChain.then(exec, exec)
	runChain = result.catch(() => {})
	return result
}
