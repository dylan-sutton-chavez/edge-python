import { chromium } from "npm:playwright@latest";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { Buffer } from "node:buffer";

const ROOT = new URL("../", import.meta.url).pathname;
const CDN_HOST = "cdn.edgepython.com";
// The staged CDN this run tests, a tmp prefix in CI or the local one from infra.
const BASE = Deno.env.get("EDGE_CDN_BASE")?.replace(/\/$/, "");
if (!BASE) throw new Error("set EDGE_CDN_BASE (npm run cdn:local in infra)");
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

/* The official origin answers from BASE, decoded bytes and the CDN's own headers, CORS included. */
async function cdn(route, url) {
    const res = await fetch(BASE + url.pathname + url.search);
    const headers = Object.fromEntries([...res.headers].filter(([k]) => k !== "content-encoding" && k !== "content-length"));
    return route.fulfill({ status: res.status, headers, body: Buffer.from(await res.arrayBuffer()) });
}

// Agnostic driver, feeds each <pkg>/<pkg>.json corpus to the <edge-python> tag. Run with deno test --allow-all harness/
async function runPackage(pkg) {
    const dir = `${ROOT}${pkg}`;
    const cases = JSON.parse(readFileSync(`${dir}/${pkg}.json`, "utf-8"));
    // Every std is declared at its CDN url, the package under test included.
    const imports = Object.fromEntries(STD.map((name) => [name, `https://${CDN_HOST}/std/${name}.${name === "test" ? "py" : "wasm"}`]));
    const manifest = existsSync(`${dir}/edge.json`)
        ? readFileSync(`${dir}/edge.json`, "utf-8")
        : JSON.stringify({ imports });

    const browser = await chromium.launch();
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push(e.message));

    /* Serve the harness page and the synthesized manifest, any host but the CDN fails the test. */
    const strays = new Set();
    await page.route("**/*", (route) => {
        const url = new URL(route.request().url());
        if (url.host === CDN_HOST) return cdn(route, url);
        if (url.host !== "localhost") {
            strays.add(`unexpected request to ${url.href}`);
            return route.abort();
        }
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
        // A stray request explains any failure it caused, report it first.
        throw strays.size ? new Error([...strays].join("\n"), { cause: e }) : e;
    } finally {
        await browser.close();
    }

    if (strays.size) failures.unshift(...strays);
    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

for (const pkg of packages) {
    Deno.test(`stdpkg: ${pkg}`, () => runPackage(pkg));
}
