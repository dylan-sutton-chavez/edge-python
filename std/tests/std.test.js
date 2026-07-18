/* Agnostic driver: feeds each <pkg>/<pkg>.json corpus to the <edge-python> tag. Run: deno test --allow-all tests/ */

import { chromium } from "npm:playwright@latest";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const MANIFEST = "/_packages.json"; // synthesized; keeps the agnostic <pkg>/ folder free of test artifacts

/* Repo-root dirs with a `<name>/<name>.json` corpus are stdpkgs. `STDPKG=<name>` narrows discovery to one package, used by the matrix-fanned CI to isolate per-shard work. */
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

async function runPackage(pkg) {
    const dir = `${ROOT}${pkg}`;
    // Import the package's `.py` entry when it has one, else the built wasm.
    const hasPy = existsSync(`${dir}/src/entry.py`);

    let entry;
    if (hasPy) {
        entry = `/${pkg}/src/entry.py`;
    } else {
        // Artifact name can differ from the dir (e.g. `struct` is a Rust keyword); any single .wasm in release/ counts.
        const releaseDir = `${dir}/target/wasm32-unknown-unknown/release`;
        const wasmName = existsSync(`${releaseDir}/${pkg}.wasm`)
            ? `${pkg}.wasm`
            : (existsSync(releaseDir) ? readdirSync(releaseDir).find((f) => f.endsWith(".wasm")) : undefined);
        if (!wasmName) {
            throw new Error(`built artifact not found for '${pkg}' in ${releaseDir}\nrun (from ${pkg}/): cargo build --release --target wasm32-unknown-unknown`);
        }
        entry = `/${pkg}/target/wasm32-unknown-unknown/release/${wasmName}`;
    }

    const cases = JSON.parse(readFileSync(`${dir}/${pkg}.json`, "utf-8"));
    // The tag's packages.json: a package may pin its own (e.g. cross-package corpora), else synthesized keyed by name.
    const manifest = existsSync(`${dir}/packages.json`)
        ? readFileSync(`${dir}/packages.json`, "utf-8")
        : JSON.stringify({ imports: { [pkg]: entry } });

    const browser = await chromium.launch();
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push(e.message));

    /* Serve repo files from disk; synthesize the manifest. External CDNs (cdn.edgepython.com) pass through. */
    await page.route("**/*", (route) => {
        const url = new URL(route.request().url());
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
        await page.goto("http://localhost/tests/index.html");
        // Boot the tag once without an entry, reuse its worker, and capture stdout via onOutput.
        await page.evaluate(async (manifestPath) => {
            const el = document.createElement("edge-python");
            el.setAttribute("packages", manifestPath);
            const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
            document.head.appendChild(el);
            await ready;
            // Byte-stream stdout: one chunk per print() call (body + its `end`); collect verbatim.
            globalThis.chunks = [];
            el.onOutput((chunk) => { globalThis.chunks.push(chunk); });
            globalThis.el = el;
        }, MANIFEST);

        for (const [i, c] of cases.entries()) {
            const src = `from ${pkg} import *\n${c.src}`;
            const result = await page.evaluate(async (s) => {
                globalThis.chunks = [];
                const { out } = await globalThis.el.run(s);
                // One entry per print() call; drop its single trailing newline (the `end`).
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
    } finally {
        await browser.close();
    }

    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

for (const pkg of packages) {
    Deno.test(`stdpkg: ${pkg}`, () => runPackage(pkg));
}
