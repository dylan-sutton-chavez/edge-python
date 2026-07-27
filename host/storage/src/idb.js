/* IndexedDB async handlers; the worker parks the coro on the returned Promise so Python sees `idb_*` as yielding builtins composing with `gather` / `with_timeout`. */

/* IDBRequest -> Promise. Native IndexedDB is callback-only; this is the standard one-liner wrapper. */
const promisify = (req) => new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
});

export default ({ dbs, allocDb, db }) => ({
    /* `idb_open(name, version, schema_json?)`, `schema_json` accepts `{"stores": ["name1", "name2"]}`; missing stores are created on upgrade. Returns a handle. */
    idb_open: async (name, version, schemaJson) => {
        const schema = schemaJson !== undefined ? JSON.parse(schemaJson || '{}') : {};
        const stores = schema.stores ?? [];
        const req = indexedDB.open(name, version);
        req.onupgradeneeded = () => {
            const idb = req.result;
            for (const store of stores) {
                if (!idb.objectStoreNames.contains(store)) idb.createObjectStore(store);
            }
        };
        return allocDb(await promisify(req));
    },

    /* Values cross the worker as JSON strings; `value_json` is parsed before put, get serializes back. */
    idb_put: async (h, store, key, valueJson) => {
        const tx = db(h).transaction(store, 'readwrite');
        await promisify(tx.objectStore(store).put(JSON.parse(valueJson), key));
    },

    /* Returns the stored value as a JSON string, or `null` if the key is missing. */
    idb_get: async (h, store, key) => {
        const tx = db(h).transaction(store, 'readonly');
        const value = await promisify(tx.objectStore(store).get(key));
        return value === undefined ? null : JSON.stringify(value);
    },

    idb_delete: async (h, store, key) => {
        const tx = db(h).transaction(store, 'readwrite');
        await promisify(tx.objectStore(store).delete(key));
    },

    /* JSON array of keys for the given store. */
    idb_keys: async (h, store) => {
        const tx = db(h).transaction(store, 'readonly');
        return JSON.stringify(await promisify(tx.objectStore(store).getAllKeys()));
    },

    /* Closes the database connection and nulls the handle. */
    idb_close: (h) => {
        const idb = dbs[h];
        if (idb) { idb.close(); dbs[h] = null; }
    },
});
