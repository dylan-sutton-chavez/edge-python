import { makeState } from './state.js';
import http from './http.js';
import ws from './ws.js';
import sse from './sse.js';

/* Factory consumed by `createWorker({ mainThreadModules: { network } })`. Composes all handler slices over a shared `state`. */
export const network = (ctx) => {
    const state = makeState();
    return Object.assign(
        {},
        http(state),
        ws(state, ctx),
        sse(state, ctx),
    );
};

export default network;
