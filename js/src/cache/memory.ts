/* In-memory cache backend, same shape as `cache/idb.ts`. Used when `integrity:false` or IDB unavailable. Methods are sync but callers `await` uniformly, keeping it interchangeable with `IdbCache`. */
import type { CacheBackend } from './types.ts';

export class MemoryCache implements CacheBackend {
    readonly persistent = false;
    cas = new Map<string, Uint8Array>(); // hash -> bytes
    lockfile = new Map<string, string>(); // spec -> hash

    open() { /* no-op */ }

    getBytes(hash: string): Uint8Array | null {
        return this.cas.get(hash) ?? null;
    }

    putBytes(hash: string, bytes: Uint8Array): void {
        this.cas.set(hash, bytes);
    }

    loadLockfile(): Map<string, string> {
        return new Map(this.lockfile);
    }

    saveLockfile(entries: Iterable<[string, string]>): void {
        for (const [k, v] of entries) this.lockfile.set(k, v);
    }

    clear(): void {
        this.cas.clear();
        this.lockfile.clear();
    }

    setVersion(_version?: string | null): void { /* no-op, nothing to invalidate across sessions */ }

    getVersion(): string | null { return null; }
}
