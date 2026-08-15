import type { CacheBackend } from './types.ts';

const IDB_NAME = 'edgepython';
const IDB_VER = 1;
const VERSION_KEY = '\0v'; // '\0' isolates sentinel, canonical specs never contain null bytes

/* IndexedDB cache with `cas` (hash -> bytes) and `lockfile` (spec -> hash) stores. The engine falls back to MemoryCache if `open()` rejects. */
export class IdbCache implements CacheBackend {
    db: IDBDatabase | null = null;

    async open(): Promise<void> {
        this.db = await new Promise<IDBDatabase>((resolve, reject) => {
            const req = self.indexedDB.open(IDB_NAME, IDB_VER);
            req.onupgradeneeded = () => {
                const db = req.result;
                if (!db.objectStoreNames.contains('cas')) db.createObjectStore('cas');
                if (!db.objectStoreNames.contains('lockfile')) db.createObjectStore('lockfile');
            };
            req.onsuccess = () => resolve(req.result);
            req.onerror = () => reject(req.error);
        });
    }

    _tx(store: string, mode: IDBTransactionMode): IDBObjectStore {
        return (this.db as IDBDatabase).transaction(store, mode).objectStore(store);
    }

    _req<T>(req: IDBRequest<T>): Promise<T> {
        return new Promise((res, rej) => {
            req.onsuccess = () => res(req.result);
            req.onerror = () => rej(req.error);
        });
    }

    getBytes(hash: string): Promise<Uint8Array | null> {
        return this._req(this._tx('cas', 'readonly').get(hash));
    }

    putBytes(hash: string, bytes: Uint8Array): Promise<void> {
        return this._req(this._tx('cas', 'readwrite').put(bytes, hash)).then(() => undefined);
    }

    async loadLockfile(): Promise<Map<string, string>> {
        const out = new Map<string, string>();
        await new Promise<void>((res, rej) => {
            const r = this._tx('lockfile', 'readonly').openCursor();
            r.onsuccess = () => {
                const c = r.result;
                if (!c) { res(); return; }
                if (c.key !== VERSION_KEY) out.set(c.key as string, c.value);
                c.continue();
            };
            r.onerror = () => rej(r.error);
        });
        return out;
    }

    async saveLockfile(entries: Iterable<[string, string]>): Promise<void> {
        const s = this._tx('lockfile', 'readwrite');
        let last: IDBRequest | undefined;
        for (const [k, v] of entries) last = s.put(v, k);
        if (last) await this._req(last);
    }

    async clear(): Promise<void> {
        await this._req(this._tx('cas', 'readwrite').clear());
        await this._req(this._tx('lockfile', 'readwrite').clear());
    }

    async setVersion(version?: string | null): Promise<void> {
        if (version) await this._req(this._tx('lockfile', 'readwrite').put(version, VERSION_KEY));
    }

    getVersion(): Promise<string | null> {
        return this._req(this._tx('lockfile', 'readonly').get(VERSION_KEY));
    }
}
