import { decodeBundle } from './bundle.ts';
import { fetchWithLockfile, requestUrl } from './fetch.ts';
import { loadNativeModule, nativeTable } from './native.ts';
import type { NativeLoader } from './native.ts';
import { dirOf, joinRel, parentDir } from './specs.ts';
import type { CompilerExports } from './wasm.ts';
import type { CacheBackend } from './cache/types.ts';
import type { Rt } from './rt.ts';
import { errMsg, writeBytes } from './util.ts';

const TD = new TextDecoder();
const TE = new TextEncoder();

// Import kinds emitted by the compiler, bare / importer-relative / root-relative.
interface ImportRecord {
    kind: 'b' | 'r' | 'R'
    spec: string
}

export interface PrefetchCtx {
    fetchedSources: Map<string, Uint8Array>
    knownMissing: Set<string>
    importsMap?: Record<string, string> | null
    mainThreadSpecs?: Set<string>
    entryDir: string
    cache: CacheBackend
    baseUrl?: string | null
    integrityActive: boolean
    loaders: NativeLoader[]
    compilerExports: CompilerExports
    rt: Rt
    loadSystem: (url: string, label: string) => Promise<string[]>
    registerSystem: (spec: string, exportNames: string[], url: string) => void
}

/* The last segment's extension without query or fragment, it picks how the artifact loads. */
const extOf = (spec: string): string => {
    const path = spec.replace(/[?#].*$/, '');
    const dot = path.lastIndexOf('.');
    return dot > path.lastIndexOf('/') ? path.slice(dot) : '';
};

// Every wasm binary opens with these four bytes.
const isWasm = (b: Uint8Array): boolean => b[0] === 0x00 && b[1] === 0x61 && b[2] === 0x73 && b[3] === 0x6d;

/* Hint when a module spec likely can't load, insecure scheme or schemeless URL. Null when it looks fine. */
function schemeHint(spec: string): string | null {
    if (spec.startsWith('http://')) {
        return `'${spec}' uses http://; browsers block http subresources from an https page `
             + `(mixed content), so the fetch never leaves. Use https:// (an SSL connection).`;
    }
    // No scheme but a dotted first segment looks like a domain, yet the host treats it as a relative path.
    const relative = spec.startsWith('.') || spec.startsWith('/') || spec.includes('://');
    const firstSegment = spec.split('/')[0];
    if (!relative && firstSegment !== undefined && firstSegment.includes('.')) {
        return `'${spec}' has no scheme, so it resolved as a path on your own origin. `
             + `If it's a URL, prefix it with https://.`;
    }
    return null;
}

/* Imports of `src`, classified, via the compiler (single source of truth). Returns [{ kind, spec }] with kind b/r/R. */
function scanImports(src: string, exports: CompilerExports): ImportRecord[] {
    if (typeof exports.extract_imports !== 'function') {
        throw new Error('compiler is missing extract_imports; runtime and wasm are out of sync');
    }
    const bytes = TE.encode(src);
    const ptr = writeBytes(exports, bytes);
    const outLen = exports.extract_imports(ptr, bytes.length);
    exports.wasm_free(ptr, Math.max(1, bytes.length));
    if (!outLen) return [];
    const text = TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), outLen));
    return text.split('\n').filter(Boolean).map((line) => ({
        kind: line[0] as ImportRecord['kind'],
        spec: line.slice(line.indexOf('\t') + 1),
    }));
}

/* Lazy BFS prefetch, bare names resolve through programmatic imports then edge.json, only used imports get fetched. */
export async function bfsPrefetch(rootSrc: string, exports: CompilerExports, lockfile: Map<string, string>, ctx: PrefetchCtx): Promise<void> {
    const { fetchedSources, knownMissing, importsMap, mainThreadSpecs, entryDir } = ctx;
    const visited = new Set<string>();
    const queue: string[] = [];
    // Module specs that never registered, thrown together at the end so the user sees a clear cause.
    const failures: string[] = [];
    // Bare name to canonical spec, programmatic imports join the synthetic root, manifest entries their own dir.
    const table: Record<string, string> = Object.fromEntries(Object.entries(importsMap || {}).map(([name, target]) => [name, joinRel('', target)]));
    // Bare names scanned before a manifest declared them, retried after each manifest merge.
    const pendingBare = new Set<string>();
    // Root-relative imports waiting on their importer's manifest chain to finish probing.
    const pendingRoot: { spec: string, dir: string }[] = []; // { spec, dir }
    const manifestDirs = new Set<string>(); // dirs whose edge.json fetched successfully
    const labels = new Map<string, string>(); // spec -> the name its first importer wrote, host-call errors show it
    const push = (spec: string, label: string): void => {
        if (!labels.has(spec)) labels.set(spec, label);
        queue.push(spec);
    };

    // Probe every ancestor manifest, mirroring the compiler walk-up.
    const enqueueManifestChain = (dir: string | null): void => {
        for (; dir != null; dir = parentDir(dir)) {
            const m = dir + 'edge.json';
            if (!knownMissing.has(m)) queue.push(m);
        }
    };

    /* Nearest dir at or above `dir` with a fetched manifest, undefined while probes are pending, null once fully probed bare. */
    const rootFor = (dir: string | null): string | null | undefined => {
        for (let d = dir; d != null; d = parentDir(d)) {
            const m = d + 'edge.json';
            if (manifestDirs.has(d)) return d;
            if (!visited.has(m) && !knownMissing.has(m)) return undefined;
        }
        return null;
    };
    const enqueueRoot = (spec: string, dir: string): void => {
        const root = rootFor(dir);
        if (root === undefined) { pendingRoot.push({ spec, dir }); return; }
        if (root !== null) push(joinRel(root, spec), spec); // null means no manifest anywhere, the compiler reports it
    };
    const retryRoot = (): void => {
        for (let i = pendingRoot.length - 1; i >= 0; i--) {
            const item = pendingRoot[i];
            if (!item || rootFor(item.dir) === undefined) continue;
            pendingRoot.splice(i, 1);
            enqueueRoot(item.spec, item.dir);
        }
    };

    /* A scanned import contributes at most one fetch target, paths queue directly, bare resolves via the table. */
    const enqueueImport = (imp: ImportRecord, dir: string): void => {
        if (imp.kind === 'r') { push(joinRel(dir, imp.spec), imp.spec); return; }
        if (imp.kind === 'R') { enqueueRoot(imp.spec, dir); return; }
        const target = table[imp.spec];
        if (target !== undefined) push(target, imp.spec);
        else pendingBare.add(imp.spec); // a later manifest may declare it
    };
    const retryPending = (): void => {
        for (const name of [...pendingBare]) {
            const target = table[name];
            if (target !== undefined) { push(target, name); pendingBare.delete(name); }
        }
    };

    // Synthetic root edge.json so the COMPILER resolves bare names at parse time the same way.
    if (Object.keys(table).length > 0) {
        fetchedSources.set('edge.json', TE.encode(JSON.stringify({ imports: table })));
        knownMissing.delete('edge.json');
    }

    // Root imports resolve from the entry's directory, like any module.
    for (const imp of scanImports(rootSrc, exports)) enqueueImport(imp, entryDir);
    enqueueManifestChain(entryDir);

    while (queue.length) {
        const spec = queue.shift();
        if (spec === undefined) break;
        if (visited.has(spec)) continue;
        visited.add(spec);

        // An inline page module (programmatic object) already registered before prefetch, nothing to fetch.
        if (mainThreadSpecs && mainThreadSpecs.has(spec)) continue;

        // JavaScript runs on the page, which imports it and returns the export names to register as stubs.
        const ext = extOf(spec);
        if (ext === '.js' || ext === '.mjs') {
            if (spec.includes('#sha256-')) { failures.push(`'${spec}' is a JavaScript module, the page imports it and cannot check a #sha256- pin`); continue; }
            const url = requestUrl(spec, ctx.baseUrl);
            let exportNames: string[];
            try { exportNames = await ctx.loadSystem(url, labels.get(spec) ?? spec); }
            catch (e) { failures.push(`'${spec}' failed to load as a JavaScript module: ${errMsg(e)}`); continue; }
            ctx.registerSystem(spec, exportNames, url);
            mainThreadSpecs?.add(spec);
            continue;
        }

        let bytes = fetchedSources.get(spec);
        if (bytes === undefined) {
            const fetched = await fetchWithLockfile(spec, lockfile, ctx);
            if (!fetched) {
                // edge.json probes are opportunistic 404s, only a real module import is worth flagging.
                if (!spec.endsWith('edge.json')) failures.push(schemeHint(spec) ?? `could not fetch module '${spec}'`);
                retryRoot(); // a settled probe may unblock a root-relative import
                continue;
            }
            bytes = fetched;
            fetchedSources.set(spec, bytes);
        }

        // A published package is verified whole, its files answer from inside it and its entry runs as the module.
        if (ext === '.edge') {
            let bundle;
            try { bundle = decodeBundle(bytes); }
            catch (e) { failures.push(`'${spec}' is not a packed .edge: ${errMsg(e)}`); continue; }
            const base = dirOf(spec);
            for (const [path, file] of bundle.files) fetchedSources.set(base + path, file);
            const entry = bundle.files.get(bundle.entry);
            if (!entry) { failures.push(`'${spec}' names an entry it does not carry`); continue; }
            bytes = entry;
        }

        if (spec.endsWith('edge.json')) {
            let parsed: { imports?: Record<string, string>, system?: unknown, extends?: string };
            try { parsed = JSON.parse(TD.decode(bytes)); }
            catch { retryRoot(); continue; }
            const dir = dirOf(spec);
            manifestDirs.add(dir);
            // A leftover `system` section merges nothing, the compiler rejects the manifest when a bare import reaches it.
            if (parsed.system !== undefined) { retryRoot(); continue; }
            // Merge as a resolution table (nearer manifests already in `table` win), then resolve any deferred names.
            for (const [name, target] of Object.entries(parsed.imports || {})) {
                if (!(name in table)) table[name] = joinRel(dir, target);
            }
            retryPending();
            retryRoot();
            if (parsed.extends) {
                const extDir = joinRel(dir, parsed.extends);
                queue.push((extDir.endsWith('/') ? extDir : extDir + '/') + 'edge.json');
            }
            continue;
        }

        // Unless the spec ends in .py, the wasm magic marks a native module, the rest is Python.
        if (ext === '.wasm' || (ext !== '.py' && isWasm(bytes))) {
            let names: string[], fns;
            try {
                ({ names, fns } = await loadNativeModule(spec, bytes, ctx));
            } catch (e) {
                // Bytes fetched but the module won't load (bad ABI / corrupt wasm), a scheme issue would have failed at fetch.
                failures.push(`'${spec}' failed to load as a wasm module: ${errMsg(e)}`);
                continue;
            }
            const baseId = nativeTable.length;
            for (const fn of fns) nativeTable.push(fn);

            const specBytes = TE.encode(spec);
            const namesBytes = TE.encode(names.join('\n'));
            exports.register_native_module(
                writeBytes(exports, specBytes), specBytes.length,
                writeBytes(exports, namesBytes), namesBytes.length,
                baseId,
            );
            enqueueManifestChain(dirOf(spec));
            continue;
        }

        // Python source, register, then scan ITS imports (bare + path) so transitive deps stay lazy too.
        const specBytes = TE.encode(spec);
        exports.register_code_module(writeBytes(exports, specBytes), specBytes.length, writeBytes(exports, bytes), bytes.length);

        const dir = dirOf(spec);
        for (const imp of scanImports(TD.decode(bytes), exports)) enqueueImport(imp, dir);
        enqueueManifestChain(dir);
    }

    if (failures.length) {
        throw new Error(`could not pre-fetch every imported module:\n  ${failures.join('\n  ')}`);
    }
    // Unresolved bare names are left to the compiler's parse-time resolver, which emits the precise error.
}
