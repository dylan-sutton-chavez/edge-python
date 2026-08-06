/* Shared handle tables. One `makeState()` per `createWorker` so multiple workers don't share open IndexedDB connections. */

export const makeState = () => {
    const dbs = [];

    const allocDb = (idb) => { dbs.push(idb); return dbs.length - 1; };
    const db = (h) => {
        if (h < 0 || h >= dbs.length || dbs[h] === null) {
            throw new Error('invalid db handle: ' + h);
        }
        return dbs[h];
    };

    return { dbs, allocDb, db };
};
