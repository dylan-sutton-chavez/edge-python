/* Shapes crossing the main-thread/worker postMessage boundary. Plain data only, no DOM or WebWorker globals, so both the `dom` and `webworker` lib scopes can import it without mixing libs in one program. */

export interface LoadOpts {
    wasmUrl: string
    integrity?: boolean
    loaders?: string[]
    imports?: Record<string, string> | null
    version?: string | null
    availableSystems?: string[]
}

export interface MainThreadManifest {
    name: string
    exports: string[]
}

export interface RunOpts {
    src: string
    repl?: boolean
    entryDir?: string
    baseUrl?: string | null
    onLine?: (text: string) => void
    incremental?: boolean
}

export interface ExecResult {
    out: string
    ms: number
    exitCode?: number
}
