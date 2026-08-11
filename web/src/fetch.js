/*
CAS-backed fetch keyed by lockfile hash; else fetch + hash + store. Null on 404 (opportunistic ok), throws on drift.
*/

import { sha256Hex } from './specs.js';

export async function fetchWithLockfile(spec, lockfile, ctx) {
    const { cache, baseUrl, knownMissing, integrityActive } = ctx;

    // An explicit #sha256- fragment pins the bytes; it stays in the cache key but leaves the request URL.
    const fragAt = spec.indexOf('#sha256-');
    const pin = fragAt === -1 ? null : spec.slice(fragAt + 8);
    const target = fragAt === -1 ? spec : spec.slice(0, fragAt);

    if (integrityActive) {
        const expected = lockfile.get(spec);
        if (expected) {
            const cached = await cache.getBytes(expected);
            if (cached) return new Uint8Array(cached);
        }
    }

    let resp;
    try {
        // Specs are root-relative; the URL join clamps escapes at the origin.
        const url = target.includes('://') ? target : new URL(target, baseUrl ?? self.location.href).toString();
        resp = await fetch(url);
    } catch (e) {
        console.warn(`[edge-python] fetch failed for '${spec}':`, e);
        return null;
    }

    if (!resp.ok) {
        if (resp.status === 404 && spec.endsWith('packages.json')) knownMissing.add(spec);
        else console.warn(`[edge-python] ${resp.status} for '${spec}' at ${resp.url}`);
        return null;
    }

    // A .wasm answered with HTML/text is a schemeless spec resolved relative and hitting an SPA fallback, not a module.
    if (target.endsWith('.wasm')) {
        const ct = (resp.headers.get('content-type') || '').toLowerCase();
        if (ct.includes('html') || ct.startsWith('text/')) {
            console.warn(`[edge-python] '${spec}' served as '${ct || 'no content-type'}', not a wasm module`);
            return null;
        }
    }

    const bytes = new Uint8Array(await resp.arrayBuffer());

    if (pin) {
        const hash = await sha256Hex(bytes);
        if (pin !== hash) {
            throw new Error(`[edge-python] integrity check failed for '${target}'\n expected sha256-${pin}\n got sha256-${hash}`);
        }
    }

    if (integrityActive) {
        const hash = await sha256Hex(bytes);
        const expected = lockfile.get(spec);
        if (expected && expected !== hash) {
            throw new Error(`[edge-python] integrity drift for '${spec}'\n  locked: sha256-${expected}\n  remote: sha256-${hash}`);
        }
        await cache.putBytes(hash, bytes);
        lockfile.set(spec, hash);
    }

    return bytes;
}
