/* Shared handle tables. One `makeState()` per `createWorker` so multiple workers don't share connections. */

export const makeState = () => {
    const requests = []; // AbortController per in-flight fetch; nulled on completion or abort.
    const sockets = []; // WebSocket per ws_open; nulled on close.
    const sseSources = []; // EventSource per sse_open; nulled on close.

    const allocSocket = (ws) => { sockets.push(ws); return sockets.length - 1; };
    const socket = (h) => {
        if (h < 0 || h >= sockets.length || sockets[h] === null) {
            throw new Error('invalid socket handle: ' + h);
        }
        return sockets[h];
    };

    const allocSse = (es) => { sseSources.push(es); return sseSources.length - 1; };
    const sse = (h) => {
        if (h < 0 || h >= sseSources.length || sseSources[h] === null) {
            throw new Error('invalid sse handle: ' + h);
        }
        return sseSources[h];
    };

    return { requests, sockets, sseSources, allocSocket, socket, allocSse, sse };
};
