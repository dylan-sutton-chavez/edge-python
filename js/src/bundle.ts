const TD = new TextDecoder();

// Mirrors `cli/src/pack.rs`, the leading bytes and the caps a hostile bundle meets.
const MAGIC = [0x45, 0x44, 0x47, 0x45, 0x50, 0x4b, 0x47, 0x01];
const MAX_FILES = 4096;
const MAX_TOTAL = 64 << 20;

export interface Bundle {
    entry: string
    files: Map<string, Uint8Array>
}

// Every path stays a plain relative one, so no file lands outside the package it came in.
const plain = (path: string): boolean =>
    path !== '' && !path.startsWith('/') && !path.includes('\\') && path.split('/').every((part) => part !== '' && part !== '.' && part !== '..');

/* A packed `.edge`, its entry and every file it carries, each length an ascii number and a newline. */
export function decodeBundle(buf: Uint8Array): Bundle {
    let p = 0;
    const take = (n: number): Uint8Array => {
        if (p + n > buf.length) throw new Error('bundle truncated');
        const slice = buf.subarray(p, p + n);
        p += n;
        return slice;
    };
    const size = (): number => {
        const end = buf.indexOf(0x0a, p);
        if (end === -1) throw new Error('bundle truncated reading a length');
        const text = TD.decode(buf.subarray(p, end));
        p = end + 1;
        if (!/^\d+$/.test(text)) throw new Error(`bundle length '${text}' is not a number`);
        return Number(text);
    };
    const path = (): string => {
        const text = TD.decode(take(size()));
        if (!plain(text)) throw new Error(`bundle path '${text}' is not a plain relative path`);
        return text;
    };

    if (!MAGIC.every((byte, i) => buf[i] === byte)) throw new Error('not an edge package');
    p = MAGIC.length;
    const entry = path();
    const count = size();
    if (count > MAX_FILES) throw new Error(`bundle has ${count} files, over the ${MAX_FILES} cap`);

    const files = new Map<string, Uint8Array>();
    let total = 0;
    for (let i = 0; i < count; i++) {
        const name = path();
        const bytes = take(size());
        total += bytes.length;
        if (total > MAX_TOTAL) throw new Error(`bundle exceeds the ${MAX_TOTAL} byte cap`);
        files.set(name, bytes);
    }
    return { entry, files };
}
