/*
IndexedDB cache: `cas` (hash -> bytes) + `lockfile` (spec -> hash). Engine falls back to MemoryCache if `open()` rejects.
*/

const IDB_NAME = 'edgepython';
const IDB_VER = 1;
const VERSION_KEY = '\0v'; // '\0' isolates sentinel, canonical specs never contain null bytes

export class IdbCache {
    constructor() {
        this.db = null;
    }

    async open() {
        this.db = await new Promise((resolve, reject) => {
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

    _tx(store, mode) {
        return this.db.transaction(store, mode).objectStore(store);
    }

    _req(req) {
        return new Promise((res, rej) => {
            req.onsuccess = () => res(req.result);
            req.onerror = () => rej(req.error);
        });
    }

    getBytes(hash) {
        return this._req(this._tx('cas', 'readonly').get(hash));
    }

    putBytes(hash, bytes) {
        return this._req(this._tx('cas', 'readwrite').put(bytes, hash));
    }

    async loadLockfile() {
        const out = new Map();
        await new Promise((res, rej) => {
            const r = this._tx('lockfile', 'readonly').openCursor();
            r.onsuccess = () => {
                const c = r.result;
                if (!c) return res();
                if (c.key !== VERSION_KEY) out.set(c.key, c.value);
                c.continue();
            };
            r.onerror = () => rej(r.error);
        });
        return out;
    }

    async saveLockfile(entries) {
        const s = this._tx('lockfile', 'readwrite');
        let last;
        for (const [k, v] of entries) last = s.put(v, k);
        if (last) await this._req(last);
    }

    async clear() {
        await this._req(this._tx('cas', 'readwrite').clear());
        await this._req(this._tx('lockfile', 'readwrite').clear());
    }

    async setVersion(version) {
        if (version) await this._req(this._tx('lockfile', 'readwrite').put(version, VERSION_KEY));
    }

    getVersion() {
        return this._req(this._tx('lockfile', 'readonly').get(VERSION_KEY));
    }
}
