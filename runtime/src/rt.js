/*
Handle-codec helpers wrapping `host_edge_decode` / `host_edge_encode`; passed to capability loaders so handlers skip NaN-boxing.
*/

const TD = new TextDecoder();
const TE = new TextEncoder();

// wasm-abi/src/lib.rs `pub mod tag`.
const TAG_NONE = 0;
const TAG_BOOL = 1;
const TAG_INT = 2;
const TAG_FLOAT = 3;
const TAG_BYTES = 4;
const TAG_RAW = 5;
const TAG_LIST = 6;
const TAG_DICT = 7;

const U64_MASK = (1n << 64n) - 1n;
const I128_MAX = (1n << 127n) - 1n;
const I128_MIN = -(1n << 127n);
const SAFE_MAX = BigInt(Number.MAX_SAFE_INTEGER);

// i128 LE read; BigInt past 2^53.
function readI128(v, off) {
    const big = (v.getBigInt64(off + 8, true) << 64n) | v.getBigUint64(off, true);
    return big >= -SAFE_MAX && big <= SAFE_MAX ? Number(big) : big;
}

// i128 LE write; throws beyond i128.
function writeI128(v, off, n) {
    const big = BigInt(n);
    if (big > I128_MAX || big < I128_MIN) throw new RangeError(`int out of i128 range: ${n}`);
    v.setBigUint64(off, big & U64_MASK, true);
    v.setBigInt64(off + 8, big >> 64n, true);
}

export function makeRt(getExports) {
    return {
        decodeStr: (h) => decodeStr(getExports(), h),
        decodeInt: (h) => decodeInt(getExports(), h),
        decodeBool: (h) => decodeBool(getExports(), h),
        decodeFloat: (h) => decodeFloat(getExports(), h),
        encodeStr: (s) => encodeStr(getExports(), s),
        encodeInt: (n) => encodeInt(getExports(), n),
        encodeBool: (b) => encodeBool(getExports(), b),
        encodeFloat: (f) => encodeFloat(getExports(), f),
        encodeNone: () => getExports().host_edge_encode(TAG_NONE, 0, 0),
        /* Tag-agnostic: decode any handle to a plain JS value; used by deferred host-call shuttling. */
        decodeAny: (h) => decodeAny(getExports(), h),
        /* Tag-agnostic: encode a plain JS value back into a handle. Integer numbers become INT; non-integer become FLOAT. */
        encodeAny: (v) => encodeAny(getExports(), v),
    };
}

// Shared alloc/decode/retry cycle; returns tag+bytes.
function rawDecode(exps, handle) {
    const tagPtr = exps.wasm_alloc(4);
    let cap = 256;
    let dstPtr = exps.wasm_alloc(cap);
    let n = exps.host_edge_decode(handle, tagPtr, dstPtr, cap);
    if (n < 0) {
        const needed = -n;
        exps.wasm_free(dstPtr, cap);
        cap = needed;
        dstPtr = exps.wasm_alloc(cap);
        n = exps.host_edge_decode(handle, tagPtr, dstPtr, cap);
    }
    const tag = new DataView(exps.memory.buffer).getUint32(tagPtr, true);
    exps.wasm_free(tagPtr, 4);
    const bytes = n > 0 ? new Uint8Array(exps.memory.buffer, dstPtr, n).slice() : new Uint8Array(0);
    exps.wasm_free(dstPtr, cap);
    return { tag, bytes };
}

function decodeBytes(exps, handle, expectedTag) {
    const { tag, bytes } = rawDecode(exps, handle);
    if (tag !== expectedTag) throw new Error(`expected tag ${expectedTag}, got ${tag}`);
    return bytes;
}

const decodeStr = (exps, h) => TD.decode(decodeBytes(exps, h, TAG_BYTES));
const decodeInt = (exps, h) => {
    const b = decodeBytes(exps, h, TAG_INT);
    return readI128(new DataView(b.buffer, b.byteOffset, 16), 0);
};
const decodeBool = (exps, h) => decodeBytes(exps, h, TAG_BOOL)[0] !== 0;
const decodeFloat = (exps, h) => {
    const b = decodeBytes(exps, h, TAG_FLOAT);
    return new DataView(b.buffer, b.byteOffset, 8).getFloat64(0, true);
};

function encodeStr(exps, s) {
    const bytes = TE.encode(s);
    const ptr = exps.wasm_alloc(bytes.length);
    new Uint8Array(exps.memory.buffer, ptr, bytes.length).set(bytes);
    const h = exps.host_edge_encode(TAG_BYTES, ptr, bytes.length);
    exps.wasm_free(ptr, bytes.length);
    return h;
}
function encodeInt(exps, n) {
    const buf = exps.wasm_alloc(16);
    writeI128(new DataView(exps.memory.buffer), buf, n);
    const h = exps.host_edge_encode(TAG_INT, buf, 16);
    exps.wasm_free(buf, 16);
    return h;
}
function encodeBool(exps, b) {
    const buf = exps.wasm_alloc(1);
    new Uint8Array(exps.memory.buffer, buf, 1)[0] = b ? 1 : 0;
    const h = exps.host_edge_encode(TAG_BOOL, buf, 1);
    exps.wasm_free(buf, 1);
    return h;
}
function encodeFloat(exps, f) {
    const buf = exps.wasm_alloc(8);
    new DataView(exps.memory.buffer).setFloat64(buf, f, true);
    const h = exps.host_edge_encode(TAG_FLOAT, buf, 8);
    exps.wasm_free(buf, 8);
    return h;
}

// Decode any tag to a JS value.
function decodeAny(exps, handle) {
    const { tag, bytes } = rawDecode(exps, handle);
    return decodeBody(tag, bytes);
}

/* Wire payload to JS value; LIST/DICT payloads hold nested TLV nodes (tag:u32le len:u32le payload). */
function decodeBody(tag, bytes) {
    const view = () => new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    switch (tag) {
        case TAG_NONE: return null;
        case TAG_BOOL: return bytes[0] !== 0;
        case TAG_INT: return readI128(view(), 0);
        case TAG_FLOAT: return view().getFloat64(0, true);
        case TAG_BYTES: return TD.decode(bytes);
        // Copy: nested payloads are views into the parent buffer.
        case TAG_RAW: return bytes.slice();
        case TAG_LIST: case TAG_DICT: {
            const v = view();
            const count = v.getUint32(0, true);
            let pos = 4;
            const nextNode = () => {
                const t = v.getUint32(pos, true);
                const len = v.getUint32(pos + 4, true);
                const payload = bytes.subarray(pos + 8, pos + 8 + len);
                pos += 8 + len;
                return decodeBody(t, payload);
            };
            if (tag === TAG_LIST) {
                return Array.from({ length: count }, nextNode);
            }
            const obj = {};
            for (let i = 0; i < count; i++) {
                const key = nextNode();
                obj[key] = nextNode();
            }
            return obj;
        }
        default: throw new Error(`unknown handle tag ${tag}`);
    }
}

/* JS value -> handle; chooses tag from typeof. Bigint also accepted for int. */
function encodeAny(exps, value) {
    if (value === null || value === undefined) return exps.host_edge_encode(TAG_NONE, 0, 0);
    if (typeof value === 'boolean') return encodeBool(exps, value);
    if (typeof value === 'bigint') return encodeInt(exps, value);
    if (typeof value === 'number') {
        return Number.isInteger(value) ? encodeInt(exps, value) : encodeFloat(exps, value);
    }
    if (typeof value === 'string') return encodeStr(exps, value);
    // Composites and raw bytes serialize to one TLV payload, then a single host_edge_encode.
    const { tag, body } = encodeNode(value);
    const ptr = exps.wasm_alloc(Math.max(1, body.length));
    new Uint8Array(exps.memory.buffer, ptr, body.length).set(body);
    const h = exps.host_edge_encode(tag, ptr, body.length);
    exps.wasm_free(ptr, Math.max(1, body.length));
    return h;
}

/* JS value to {tag, body}; used for nested nodes and composite roots. */
function encodeNode(value) {
    if (value === null || value === undefined) return { tag: TAG_NONE, body: new Uint8Array(0) };
    if (typeof value === 'boolean') return { tag: TAG_BOOL, body: Uint8Array.of(value ? 1 : 0) };
    if (typeof value === 'number' || typeof value === 'bigint') {
        const body = new Uint8Array(typeof value === 'number' && !Number.isInteger(value) ? 8 : 16);
        const v = new DataView(body.buffer);
        if (body.length === 8) {
            v.setFloat64(0, value, true);
            return { tag: TAG_FLOAT, body };
        }
        writeI128(v, 0, value);
        return { tag: TAG_INT, body };
    }
    if (typeof value === 'string') return { tag: TAG_BYTES, body: TE.encode(value) };
    if (value instanceof Uint8Array) return { tag: TAG_RAW, body: value };
    if (value instanceof ArrayBuffer) return { tag: TAG_RAW, body: new Uint8Array(value) };
    if (Array.isArray(value)) {
        return { tag: TAG_LIST, body: joinNodes(value.length, value.map(encodeNode)) };
    }
    if (typeof value === 'object') {
        const nodes = Object.entries(value).flatMap(([k, v]) => [encodeNode(k), encodeNode(v)]);
        return { tag: TAG_DICT, body: joinNodes(Object.keys(value).length, nodes) };
    }
    throw new Error(`cannot encode JS value of type ${typeof value}`);
}

// count:u32le then each node framed as tag:u32le len:u32le payload.
function joinNodes(count, nodes) {
    const total = 4 + nodes.reduce((n, { body }) => n + 8 + body.length, 0);
    const out = new Uint8Array(total);
    const v = new DataView(out.buffer);
    v.setUint32(0, count, true);
    let pos = 4;
    for (const { tag, body } of nodes) {
        v.setUint32(pos, tag, true);
        v.setUint32(pos + 4, body.length, true);
        out.set(body, pos + 8);
        pos += 8 + body.length;
    }
    return out;
}
