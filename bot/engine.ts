import { writeFileSync } from 'node:fs'
import { ENGINE } from './names'

// Carried inside the Worker rather than fetched by it, since a Worker cannot compile wasm while it runs.
const response = await fetch(ENGINE)
if (!response.ok) throw new Error(`${ENGINE} answered ${response.status}`)

writeFileSync(new URL('./compiler.wasm', import.meta.url), new Uint8Array(await response.arrayBuffer()))
