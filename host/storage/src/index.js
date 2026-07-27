/* Factory consumed by `createWorker({ mainThreadModules: { storage } })`. Composes all handler slices over a shared `state`. */

import { makeState } from './state.js';
import kv from './kv.js';
import idb from './idb.js';

export const storage = () => {
    const state = makeState();
    return Object.assign(
        {},
        kv(),
        idb(state),
    );
};

export default storage;
