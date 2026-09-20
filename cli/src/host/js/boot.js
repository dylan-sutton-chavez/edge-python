export const F = globalThis.fetch.bind(globalThis);
export const LOG = console.log.bind(console);
export const B64 = globalThis.btoa.bind(globalThis);

// Timers and fetches still pending, a parked receive() keeps waiting while any may push an event.
const pending = new Set();
let report = () => {};
export const watch = (fn) => { report = fn; };
const arm = (id) => { pending.add(id); report(pending.size); return id; };
const disarm = (id) => { if (pending.delete(id)) report(pending.size); };
const [setT, setI, clearT, clearI] = [globalThis.setTimeout, globalThis.setInterval, globalThis.clearTimeout, globalThis.clearInterval];
globalThis.setTimeout = (fn, ms, ...args) => {
  const id = setT((...a) => { disarm(id); if (typeof fn === "function") fn(...a); }, ms, ...args);
  return arm(id);
};
globalThis.setInterval = (fn, ms, ...args) => arm(setI(fn, ms, ...args));
globalThis.clearTimeout = (id) => { disarm(id); clearT(id); };
globalThis.clearInterval = (id) => { disarm(id); clearI(id); };
globalThis.fetch = (...args) => {
  const request = arm({});
  const settled = F(...args);
  settled.then(() => disarm(request), () => disarm(request));
  return settled;
};

let start;
const started = new Promise((resolve) => { start = resolve; });
addEventListener("fetch", (event) => {
  event.respondWith(new Response(""));
  event.waitUntil(started.then((main) => main()));
});
export const run = (main) => start(main);
