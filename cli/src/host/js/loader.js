import { F, LOG, B64, run, watch } from "./boot.js";
import * as mod from __ENTRY__;

const TOKEN = __TOKEN__;
const LABEL = __LABEL__;
const PREFIX = `${TOKEN}`;
const TE = new TextEncoder();
const TD = new TextDecoder();
const SAFE = BigInt(Number.MAX_SAFE_INTEGER);

function u32(n) { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n, true); return b; }

function concat(parts) {
  let len = 0; for (const p of parts) len += p.length;
  const out = new Uint8Array(len); let at = 0; for (const p of parts) { out.set(p, at); at += p.length; } return out;
}

// One wire node, the same tags and bodies as WireValue in abi/src/lib.rs.
function encodeNode(value, out) {
  let tag;
  let body;
  if (value === null || value === undefined) { tag = 0; body = new Uint8Array(0); }
  else if (typeof value === "boolean") { tag = 1; body = Uint8Array.of(value ? 1 : 0); }
  else if (typeof value === "bigint" || (typeof value === "number" && Number.isInteger(value))) {
    tag = 2; body = new Uint8Array(16);
    const big = BigInt(value); const view = new DataView(body.buffer);
    view.setBigUint64(0, big & 0xffffffffffffffffn, true); view.setBigInt64(8, big >> 64n, true);
  }
  else if (typeof value === "number") { tag = 3; body = new Uint8Array(8); new DataView(body.buffer).setFloat64(0, value, true); }
  else if (typeof value === "string") { tag = 4; body = TE.encode(value); }
  else if (value instanceof Uint8Array) { tag = 5; body = value; }
  else if (value instanceof ArrayBuffer) { tag = 5; body = new Uint8Array(value); }
  else if (Array.isArray(value)) {
    tag = 6; const parts = [u32(value.length)]; for (const item of value) encodeNode(item, parts); body = concat(parts);
  } else {
    tag = 7; const keys = Object.keys(value); const parts = [u32(keys.length)];
    for (const key of keys) { encodeNode(key, parts); encodeNode(value[key], parts); } body = concat(parts);
  }
  out.push(u32(tag), u32(body.length), body);
}

function decodeNode(bytes, pos) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = view.getUint32(pos.at, true); const len = view.getUint32(pos.at + 4, true);
  const start = pos.at + 8; pos.at = start + len;
  const body = bytes.subarray(start, start + len); const bv = new DataView(body.buffer, body.byteOffset, body.byteLength);
  switch (tag) {
    case 0: return null;
    case 1: return body[0] !== 0;
    case 2: { const big = (bv.getBigInt64(8, true) << 64n) | bv.getBigUint64(0, true); return big <= SAFE && big >= -SAFE ? Number(big) : big; }
    case 3: return bv.getFloat64(0, true);
    case 4: return TD.decode(body);
    case 5: return body.slice();
    case 6: { const n = bv.getUint32(0, true); const inner = { at: 4 }; const items = []; for (let i = 0; i < n; i++) items.push(decodeNode(body, inner)); return items; }
    case 7: { const n = bv.getUint32(0, true); const inner = { at: 4 }; const obj = {}; for (let i = 0; i < n; i++) { const k = decodeNode(body, inner); obj[k] = decodeNode(body, inner); } return obj; }
    default: throw new TypeError(`unknown wire tag ${tag}`);
  }
}

function send(messages) {
  const parts = []; encodeNode(messages, parts); const bytes = concat(parts);
  let text = "";
  for (let i = 0; i < bytes.length; i += 8192) text += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
  LOG(PREFIX + B64(text));
}

// One turn's messages travel as one batch, the host sees each event with the work it armed.
let queued = [];
function flush() { const messages = queued; queued = []; if (messages.length) send(messages); }
function post(messages) { if (!queued.length) queueMicrotask(flush); queued.push(...messages); }

const fault = (e) => [e && e.name ? String(e.name) : "Error", e && e.message !== undefined ? String(e.message) : String(e)];
const instances = new Map();
let outbox = [];

watch((n) => post([["busy", n]]));

function bind(id, instance) {
  try {
    const source = mod.default ?? mod[LABEL];
    if (!source) throw new Error(`no default export and no '${LABEL}' export`);
    const ctx = { pushEvent: (message) => post([["event", instance, String(message)]]) };
    const handlers = typeof source === "function" ? source(ctx) : source;
    instances.set(instance, handlers);
    outbox.push(["bound", id, Object.keys(handlers)]);
  } catch (e) { outbox.push(["error", id, ...fault(e)]); }
}

function call(id, instance, name, args) {
  let result;
  try {
    result = instances.get(instance)[name](...args);
  } catch (e) { outbox.push(["error", id, ...fault(e)]); return; }
  if (result && typeof result.then === "function") {
    outbox.push(["pending", id]);
    result.then((value) => post([["settle", id, value]]), (e) => post([["error", id, ...fault(e)]]));
  } else outbox.push(["ok", id, result]);
}

async function main() {
  const response = await F("http://edge.internal/stream", { method: "POST", headers: { "x-edge": TOKEN } });
  const reader = response.body.getReader();
  let buffer = new Uint8Array(0);
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buffer = buffer.length ? concat([buffer, value]) : value;
    while (buffer.length >= 4) {
      const len = new DataView(buffer.buffer, buffer.byteOffset, 4).getUint32(0, true);
      if (buffer.length < 4 + len) break;
      const batch = len ? decodeNode(buffer.subarray(4, 4 + len), { at: 0 }) : [];
      buffer = buffer.subarray(4 + len);
      for (const message of batch) {
        switch (message[0]) {
          case "quit": reader.cancel(); return;
          case "bind": bind(message[1], message[2]); break;
          case "call": call(message[1], message[2], message[3], message[4]); break;
          case "unbind": instances.delete(message[1]); break;
        }
      }
      if (outbox.length) { send([...queued, ...outbox]); queued = []; outbox = []; }
    }
  }
}

run(main);
