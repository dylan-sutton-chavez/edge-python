import { fetchWithLockfile } from './fetch.ts';
import { loadNativeModule, nativeTable } from './native.ts';
import type { NativeLoader } from './native.ts';
import { dirOf, joinRel, parentDir, SOURCE_LIMIT } from './specs.ts';
import type { CompilerExports } from './wasm.ts';
import type { CacheBackend } from './cache/types.ts';
import type { Rt } from './rt.ts';

const TD = new TextDecoder();
const TE = new TextEncoder();

interface ImportRecord {
    kind: string
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
    loadSystem: (name: string, url?: string) => Promise<string[]>
    registerSystem: (name: string, exportNames: string[]) => void
}

/* Hint when a module spec likely can't load, insecure scheme or schemeless URL. Null when it looks fine. */
function schemeHint(spec: string): string | null {
    if (spec.startsWith('http://')) {
        return `'${spec}' uses http://; browsers block http subresources from an https page `
             + `(mixed content), so the fetch never leaves. Use https:// (an SSL connection).`;
    }
    // No scheme but a dotted first segment looks like a domain, yet the host treats it as a relative path.
    const relative = spec.startsWith('.') || spec.startsWith('/') || spec.includes('://');
    if (!relative && spec.split('/')[0].includes('.')) {
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
    const len = Math.min(bytes.length, SOURCE_LIMIT);
    new Uint8Array(exports.memory.buffer, exports.src_ptr(), len).set(bytes.subarray(0, len));
    const outLen = exports.extract_imports(len);
    if (!outLen) return [];
    const text = TD.decode(new Uint8Array(exports.memory.buffer, exports.out_ptr(), outLen));
    return text.split('\n').filter(Boolean).map((line) => ({
        kind: line[0],
        spec: line.slice(line.indexOf('\t') + 1),
    }));
}

/* Lazy prefetch over the dependency graph. The compiler classifies each import (bare, importer-relative, root-relative), bare names resolve against the manifest chain (defaults < user packages.json), and only the imports a module actually uses get fetched. Manifests are resolution tables, not download lists. */
export async function bfsPrefetch(rootSrc: string, exports: CompilerExports, lockfile: Map<string, string>, ctx: PrefetchCtx): Promise<void> {
    const { fetchedSources, knownMissing, importsMap, mainThreadSpecs, entryDir } = ctx;
    const visited = new Set<string>();
    const queue: string[] = [];
    // Module specs that never registered, thrown together at the end so the user sees a clear cause.
    const failures: string[] = [];
    // Bare-name -> target spec. Seeded from importsMap (defaults + user), physical packages.json merge in as discovered.
    const table: Record<string, string> = { ...(importsMap || {}) };
    // Bare names scanned before a manifest declared them, retried after each manifest merge.
    const pendingBare = new Map<string, string[]>(); // name -> importer dirs, for relative targets
    // Root-relative imports waiting on their importer's manifest chain to finish probing.
    const pendingRoot: { spec: string, dir: string }[] = []; // { spec, dir }
    const manifestDirs = new Set<string>(); // dirs whose packages.json fetched successfully
    const systemEsmUrls = new Map<string, string>(); // name -> ESM url from discovered `system` declarations

    const writeBytes = (bytes: Uint8Array): number => {
        const ptr = exports.wasm_alloc(Math.max(1, bytes.length));
        new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
        return ptr;
    };
    // Probe every ancestor manifest, mirroring the compiler walk-up.
    const enqueueManifestChain = (dir: string | null): void => {
        for (; dir != null; dir = parentDir(dir)) {
            const m = dir + 'packages.json';
            if (!knownMissing.has(m)) queue.push(m);
        }
    };

    /* Nearest dir at or above `dir` with a fetched manifest, undefined while probes are pending, null once fully probed bare. */
    const rootFor = (dir: string | null): string | null | undefined => {
        for (let d = dir; d != null; d = parentDir(d)) {
            const m = d + 'packages.json';
            if (manifestDirs.has(d)) return d;
            if (!visited.has(m) && !knownMissing.has(m)) return undefined;
        }
        return null;
    };
    const enqueueRoot = (spec: string, dir: string): void => {
        const root = rootFor(dir);
        if (root === undefined) { pendingRoot.push({ spec, dir }); return; }
        if (root !== null) queue.push(joinRel(root, spec)); // null means no manifest anywhere, the compiler reports it
    };
    const retryRoot = (): void => {
        for (let i = pendingRoot.length - 1; i >= 0; i--) {
            const { spec, dir } = pendingRoot[i];
            if (rootFor(dir) === undefined) continue;
            pendingRoot.splice(i, 1);
            enqueueRoot(spec, dir);
        }
    };

    /* A scanned import contributes at most one fetch target, paths queue directly, bare resolves via the table. */
    const enqueueImport = (imp: ImportRecord, dir: string): void => {
        if (imp.kind === 'r') { queue.push(joinRel(dir, imp.spec)); return; }
        if (imp.kind === 'R') { enqueueRoot(imp.spec, dir); return; }
        const target = table[imp.spec];
        if (target !== undefined) queue.push(joinRel(dir, target));
        else { const ds = pendingBare.get(imp.spec); ds ? ds.push(dir) : pendingBare.set(imp.spec, [dir]); } // a later manifest may declare it
    };
    const retryPending = (): void => {
        for (const [name, dirs] of [...pendingBare]) {
            const target = table[name];
            if (target !== undefined) { for (const dir of dirs) queue.push(joinRel(dir, target)); pendingBare.delete(name); }
        }
    };

    // Synthetic root packages.json so the COMPILER resolves bare names at parse time the same way.
    if (Object.keys(table).length > 0) {
        fetchedSources.set('packages.json', TE.encode(JSON.stringify({ imports: table })));
        knownMissing.delete('packages.json');
    }

    // Root imports resolve from the entry's directory, like any module.
    for (const imp of scanImports(rootSrc, exports)) enqueueImport(imp, entryDir);
    enqueueManifestChain(entryDir);

    while (queue.length) {
        const spec = queue.shift() as string;
        if (visited.has(spec)) continue;
        visited.add(spec);

        // Eager system (programmatic object) already registered before prefetch, nothing to fetch.
        if (mainThreadSpecs && mainThreadSpecs.has(spec)) continue;

        // Lazy system, ask the page to load the ESM, then register its exports as `mt:<name>` stubs.
        if (spec.startsWith('mt:')) {
            const name = spec.slice(3);
            let exportNames: string[];
            try { exportNames = await ctx.loadSystem(name, systemEsmUrls.get(name)); }
            catch (e) { failures.push(`system '${name}' failed to load: ${e instanceof Error ? e.message : e}`); continue; }
            ctx.registerSystem(name, exportNames);
            mainThreadSpecs?.add(spec);
            continue;
        }

        let bytes: Uint8Array;
        if (fetchedSources.has(spec)) {
            bytes = fetchedSources.get(spec) as Uint8Array;
        } else {
            const fetched = await fetchWithLockfile(spec, lockfile, ctx);
            if (!fetched) {
                // packages.json probes are opportunistic 404s, only a real module import is worth flagging.
                if (!spec.endsWith('packages.json')) failures.push(schemeHint(spec) ?? `could not fetch module '${spec}'`);
                retryRoot(); // a settled probe may unblock a root-relative import
                continue;
            }
            bytes = fetched;
            fetchedSources.set(spec, bytes);
        }

        if (spec.endsWith('packages.json')) {
            let parsed: { imports?: Record<string, string>, system?: Record<string, string>, extends?: string };
            try { parsed = JSON.parse(TD.decode(bytes)); }
            catch { retryRoot(); continue; }
            const dir = dirOf(spec);
            manifestDirs.add(dir);
            // Merge as a resolution table (nearer manifests already in `table` win), then resolve any deferred names.
            for (const [name, target] of Object.entries(parsed.imports || {})) {
                if (!(name in table)) table[name] = joinRel(dir, target);
            }
            // `system` entries declare mt: stubs, the page imports the ESM.
            for (const [name, target] of Object.entries(parsed.system || {})) {
                if (!(name in table)) table[name] = 'mt:' + name;
                if (!systemEsmUrls.has(name)) systemEsmUrls.set(name, joinRel(dir, target));
            }
            retryPending();
            retryRoot();
            if (parsed.extends) {
                const extDir = joinRel(dir, parsed.extends);
                queue.push((extDir.endsWith('/') ? extDir : extDir + '/') + 'packages.json');
            }
            continue;
        }

        if (spec.endsWith('.wasm')) {
            let names: string[], fns;
            try {
                ({ names, fns } = await loadNativeModule(spec, bytes, ctx));
            } catch (e) {
                // Bytes fetched but the module won't load (bad ABI / corrupt wasm), a scheme issue would have failed at fetch.
                failures.push(`'${spec}' failed to load as a wasm module: ${e instanceof Error ? e.message : e}`);
                continue;
            }
            const baseId = nativeTable.length;
            for (const fn of fns) nativeTable.push(fn);

            const specBytes = TE.encode(spec);
            const namesBytes = TE.encode(names.join('\n'));
            exports.register_native_module(
                writeBytes(specBytes), specBytes.length,
                writeBytes(namesBytes), namesBytes.length,
                baseId,
            );
            enqueueManifestChain(dirOf(spec));
            continue;
        }

        // .py module, register, then scan ITS imports (bare + path) so transitive deps stay lazy too.
        const specBytes = TE.encode(spec);
        exports.register_code_module(writeBytes(specBytes), specBytes.length, writeBytes(bytes), bytes.length);

        const dir = dirOf(spec);
        for (const imp of scanImports(TD.decode(bytes), exports)) enqueueImport(imp, dir);
        enqueueManifestChain(dir);
    }

    if (failures.length) {
        throw new Error(`could not pre-fetch every imported module:\n  ${failures.join('\n  ')}`);
    }
    // Unresolved bare names are left to the compiler's parse-time resolver, which emits the precise error.
}
