/*
Built-in base manifest. Official std packages resolvable by bare name with no user packages.json.
Lowest precedence (user `imports` win) and lazy: an unused default is never fetched.
*/

/* Worker-side std packages (.wasm). Pinned for reproducibility; the lockfile verifies the bytes when integrity is on. */
export const DEFAULT_IMPORTS = {
    json: 'https://cdn.edgepython.com/std/json.wasm',
    re: 'https://cdn.edgepython.com/std/re.wasm',
    math: 'https://cdn.edgepython.com/std/math.wasm',
    struct: 'https://cdn.edgepython.com/std/struct.wasm',
    test: 'https://cdn.edgepython.com/std/test.py',
    dom: 'https://cdn.edgepython.com/web/builtins/dom/entry.py', // e.g., `dom` is a .py facade over the `_dom` system module.
};

/* Main-thread system libraries (ESM). Pages flattens each `<name>/src/` to `cdn.edgepython.com/web/builtins/<name>/`. Same lazy + opt-out rules; merged under any user `system` entries. */
export const DEFAULT_SYSTEM = {
    _dom: 'https://cdn.edgepython.com/web/builtins/dom/index.js',
    network: 'https://cdn.edgepython.com/web/builtins/network/index.js',
    storage: 'https://cdn.edgepython.com/web/builtins/storage/index.js',
    time: 'https://cdn.edgepython.com/web/builtins/time/index.js',
};
