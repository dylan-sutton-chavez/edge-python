import { chromium } from "npm:playwright@latest";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const HOST = new URL("../../js/", import.meta.url).pathname;
const DIST = HOST + "dist/"; // tsc emit of js/src, built below
const REPO = new URL("../../", import.meta.url).pathname;
const CDN_HOST = "cdn.edgepython.com";
const MANIFEST = "/_edge.json"; // synthesized, keeps the agnostic <pkg>/ folder free of test artifacts
const STD = ["json", "re", "math", "struct", "test"];

/* Dirs with a `<name>/<name>.json` corpus are stdpkgs, `STDPKG=<name>` narrows discovery to one. */
const only = Deno.env.get("STDPKG");
const packages = readdirSync(ROOT).filter((name) => {
    const dir = ROOT + name;
    if (!statSync(dir).isDirectory()) return false;
    if (only && name !== only) return false;
    return existsSync(`${dir}/${name}.json`);
});

const TYPES = {
    ".html": "text/html",
    ".js": "text/javascript",
    ".wasm": "application/wasm",
    ".json": "application/json",
    ".py": "text/plain",
};

// The artifact name can differ from the dir (`struct` is a Rust keyword), any single release .wasm counts.
function builtWasm(name) {
    const dir = `${ROOT}${name}/target/wasm32-unknown-unknown/release`;
    if (existsSync(`${dir}/${name}.wasm`)) return `${name}.wasm`;
    return existsSync(dir) ? readdirSync(dir).find((f) => f.endsWith(".wasm")) : undefined;
}

let distBuilt = false;
async function buildDist() {
    if (distBuilt) return;
    const tsc = (cfg) => new Deno.Command(Deno.execPath(), { args: ["run", "-A", "npm:typescript@5.9.3/tsc", "-p", cfg], cwd: HOST }).output();
    for (const c of ["tsconfig.json", "tsconfig.worker.json"]) {
        const r = await tsc(c);
        if (!r.success) throw new Error(`tsc failed: ${c}`);
    }
    distBuilt = true;
}

// Agnostic driver, feeds each <pkg>/<pkg>.json corpus to the <edge-python> tag. Run with deno test --allow-all harness/
async function runPackage(pkg) {
    await buildDist();
    const dir = `${ROOT}${pkg}`;
    // Import the package's `.py` entry when it has one, else the built wasm.
    const hasPy = existsSync(`${dir}/src/entry.py`);

    let entry;
    if (hasPy) {
        entry = `/${pkg}/src/entry.py`;
    } else {
        const wasmName = builtWasm(pkg);
        if (!wasmName) {
            throw new Error(`built artifact not found for '${pkg}'\nrun (from ${pkg}/): cargo build --release --target wasm32-unknown-unknown`);
        }
        entry = `/${pkg}/target/wasm32-unknown-unknown/release/${wasmName}`;
    }

    const cases = JSON.parse(readFileSync(`${dir}/${pkg}.json`, "utf-8"));
    // Every std is declared at its CDN url, the package under test points at the local build.
    const imports = Object.fromEntries(STD.map((name) => [name, `https://${CDN_HOST}/std/${name}.${name === "test" ? "py" : "wasm"}`]));
    imports[pkg] = entry;
    const manifest = existsSync(`${dir}/edge.json`)
        ? readFileSync(`${dir}/edge.json`, "utf-8")
        : JSON.stringify({ imports });

    const browser = await chromium.launch();
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push(e.message));

    /* Serve repo files and the synthesized manifest, a CDN path the tree lacks fails the test. */
    const offline = new Set();
    await page.route("**/*", (route) => {
        const url = new URL(route.request().url());
        // A manifest the tree lacks answers 404 like the deploy, any other miss fails the test.
        const miss = (hint) => {
            if (url.pathname.endsWith("/edge.json")) return route.fulfill({ status: 404 });
            offline.add(hint);
            return route.abort();
        };
        // A sibling std at its CDN url is served from its local build.
        if (url.host === CDN_HOST && url.pathname.startsWith("/std/")) {
            const name = url.pathname.slice("/std/".length).replace(/\.(wasm|py)$/, "");
            const wasm = name === "test" ? undefined : builtWasm(name);
            const local = name === "test" ? `${ROOT}test/src/entry.py` : `${ROOT}${name}/target/wasm32-unknown-unknown/release/${wasm}`;
            try { return route.fulfill({ contentType: TYPES[url.pathname.slice(url.pathname.lastIndexOf("."))], body: readFileSync(local) }); }
            catch { return miss(`build std/${name} first`); }
        }
        // In-tree wasm so new exports are testable.
        if (url.host === CDN_HOST && url.pathname === "/compiler.wasm") {
            const local = `${REPO}target/wasm32-unknown-unknown/release/compiler.wasm`;
            try { return route.fulfill({ contentType: "application/wasm", body: readFileSync(local) }); }
            catch { return miss("run cargo wasm first"); }
        }
        if (url.host === CDN_HOST && url.pathname.startsWith("/js/src/")) {
            const path = DIST + url.pathname.slice("/js/src/".length);
            try {
                return route.fulfill({ body: readFileSync(path), contentType: TYPES[path.slice(path.lastIndexOf("."))] ?? "application/octet-stream" });
            } catch {
                return miss(`js/dist has no ${url.pathname.slice("/js/src/".length)}`);
            }
        }
        // In-tree JS host first, CI must test the checkout not the deploy.
        if (url.host === CDN_HOST && url.pathname.startsWith("/js/")) {
            const path = HOST + url.pathname.slice("/js/".length);
            try {
                return route.fulfill({ body: readFileSync(path), contentType: TYPES[path.slice(path.lastIndexOf("."))] ?? "application/octet-stream" });
            } catch {
                return miss(`js${url.pathname.slice("/js".length)} is missing from the tree`);
            }
        }
        if (url.host === CDN_HOST) return miss(`no local copy of ${url.href}`);
        if (url.host !== "localhost") return route.continue();
        if (url.pathname === MANIFEST) return route.fulfill({ contentType: "application/json", body: manifest });
        const path = ROOT + url.pathname.slice(1);
        try {
            const ext = path.slice(path.lastIndexOf("."));
            return route.fulfill({ body: readFileSync(path), contentType: TYPES[ext] ?? "application/octet-stream" });
        } catch {
            return route.fulfill({ status: 404 });
        }
    });

    const failures = [];
    try {
        await page.goto("http://localhost/harness/index.html");
        // Boot the tag once without an entry, reuse its worker, and capture stdout via onOutput.
        let bootTimer;
        try {
            await Promise.race([
                page.evaluate(async (manifestPath) => {
                    const el = document.createElement("edge-python");
                    el.setAttribute("manifest", manifestPath);
                    const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
                    document.head.appendChild(el);
                    await ready;
                    // Byte-stream stdout, one chunk per print() call (body + its `end`), collected verbatim.
                    globalThis.chunks = [];
                    el.worker.onOutput((chunk) => { globalThis.chunks.push(chunk); });
                    globalThis.el = el;
                }, MANIFEST),
                new Promise((_, reject) => { bootTimer = setTimeout(
                    () => reject(new Error(`edge-python tag never fired 'ready' (page errors: ${errors.join(" | ") || "none"})`)),
                    60_000,
                ); }),
            ]);
        } finally {
            clearTimeout(bootTimer);
        }

        for (const [i, c] of cases.entries()) {
            const src = `from ${pkg} import *\n${c.src}`;
            const result = await page.evaluate(async (s) => {
                globalThis.chunks = [];
                const { out } = await globalThis.el.worker.run(s);
                // One entry per print() call. Drop its single trailing newline (the `end`).
                const output = globalThis.chunks.map((c) => c.replace(/\n$/, ""));
                return { output, error: out || null };
            }, src);

            if (c.error) {
                if (!result.error || !result.error.includes(c.error)) {
                    failures.push(`[${pkg} #${i}] expected error containing '${c.error}', got: ${result.error ?? "(none)"}`);
                }
                continue;
            }
            if (result.error) {
                failures.push(`[${pkg} #${i}] unexpected error: ${result.error}`);
                continue;
            }
            const expected = c.output ?? [];
            if (JSON.stringify(result.output) !== JSON.stringify(expected)) {
                failures.push(`[${pkg} #${i}] output mismatch\n  src: ${c.src}\n  expected: ${JSON.stringify(expected)}\n  got: ${JSON.stringify(result.output)}`);
            }
        }

        if (errors.length) failures.push(`[${pkg}] console errors: ${errors.join(" | ")}`);
    } catch (e) {
        // A tree miss explains any failure it caused, report it first.
        throw offline.size ? new Error([...offline].join("\n"), { cause: e }) : e;
    } finally {
        await browser.close();
    }

    if (offline.size) failures.unshift(...offline);
    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

for (const pkg of packages) {
    Deno.test(`stdpkg: ${pkg}`, () => runPackage(pkg));
}
