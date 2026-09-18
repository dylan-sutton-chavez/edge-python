// One Web Worker per page, loading the JS host and compiler.wasm lazily.
const WASM_URL = 'https://cdn.edgepython.com/compiler.wasm'
const JS_URL = 'https://cdn.edgepython.com/js/src/index.js'
const STD_URL = 'https://cdn.edgepython.com/std/'
const BUILTINS_URL = 'https://cdn.edgepython.com/js/builtins/'

// Nothing resolves undeclared, the playground declares every official package like any page would.
const IMPORTS = {
	json: STD_URL + 'json.wasm',
	re: STD_URL + 're.wasm',
	math: STD_URL + 'math.wasm',
	struct: STD_URL + 'struct.wasm',
	test: STD_URL + 'test.py',
	dom: BUILTINS_URL + 'dom/entry.py',
}
const SYSTEM = {
	storage: BUILTINS_URL + 'storage/index.js',
	network: BUILTINS_URL + 'network/index.js',
	time: BUILTINS_URL + 'time/index.js',
}

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

// `onPhase` only matters for the cold call, 'host' while the ESM downloads then 'worker' while it spawns.
function getWorker(onPhase) {
	if (workerPromise) return workerPromise
	workerPromise = (async () => {
		onPhase?.('host')
		// webpackIgnore keeps Next from bundling the cross-origin ESM, the browser loads it on demand.
		const { createWorker } = await import(/* webpackIgnore: true */ JS_URL)
		onPhase?.('worker')
		const worker = await createWorker({ wasmUrl: WASM_URL, integrity: true, imports: IMPORTS, systemModules: SYSTEM })
		worker.onOutput((chunk) => { if (activeSink) activeSink(chunk) })
		workerReady = true
		return worker
	})()
	return workerPromise
}

/* Runs `src`, streams stdout chunks to `onChunk`, resolves with the error text and elapsed ms. */
export async function run(src, onChunk, onPhase) {
	const exec = async () => {
		// Only the cold start drives the load phases, warm runs go straight to running.
		const worker = await getWorker(workerReady ? undefined : onPhase)
		onPhase?.('running')
		activeSink = onChunk
		let timer
		try {
			// Race against a hard timeout, terminate() kills the worker even mid loop.
			const runP = worker.run(src, { baseUrl: location.href })
			runP.catch(() => {}) // loser of the race; killWorker rejects it with nobody listening
			const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Run exceeded ${RUN_TIMEOUT_MS / 1000}s — worker terminated`)), RUN_TIMEOUT_MS) })
			const { out, ms } = await Promise.race([runP, timeout])
			return { error: out || '', ms }
		} catch (e) {
			// A timeout or a dead worker respawns so the queue never wedges.
			await killWorker()
			throw e
		} finally {
			clearTimeout(timer)
			activeSink = null
		}
	}
	// Queue behind any in-flight run so a second block never overwrites the first's sink.
	const result = runChain.then(exec, exec)
	runChain = result.catch(() => {})
	return result
}
