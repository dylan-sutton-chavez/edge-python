/* Agnostic driver: feeds each <cap>/<cap>.json corpus to the <edge-python> tag. Run: deno test --allow-all tests/ */

import { chromium } from "npm:playwright";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { DEFAULT_IMPORTS } from "../../src/defaults.js";

const ROOT = new URL("../", import.meta.url).pathname;
const RUNTIME = new URL("../../", import.meta.url).pathname;
const CDN_HOST = new URL(Object.values(DEFAULT_IMPORTS)[0]).host;
const MANIFEST = "/_packages.json"; // synthesized; keeps the agnostic <cap>/ folder free of test artifacts

/* Repo-root dirs with a `<cap>/<cap>.json` corpus are host capabilities. `HOSTCAP=<name>` narrows discovery to one capability, used by the matrix-fanned CI to isolate per-shard work. */
const only = Deno.env.get("HOSTCAP");
const capabilities = readdirSync(ROOT).filter((name) => {
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
    ".svg": "image/svg+xml",
    ".py": "text/plain",
    ".css": "text/css",
};

async function runCapability(cap) {
    const dir = `${ROOT}${cap}`;
    // Import the capability's `.py` entry when it has one, else the JS host module.
    const hasPy = existsSync(`${dir}/src/entry.py`);

    const cases = JSON.parse(readFileSync(`${dir}/${cap}.json`, "utf-8"));
    // The tag's packages.json: a capability may pin its own (e.g. python wrapper + host module pairs), else synthesized: python to entry.py as a code module; else the JS host module.
    const manifest = existsSync(`${dir}/packages.json`)
        ? readFileSync(`${dir}/packages.json`, "utf-8")
        : JSON.stringify(
            hasPy
                ? { imports: { [cap]: `/${cap}/src/entry.py` } }
                : { host: { [cap]: `/${cap}/src/index.js` } },
        );

    const browser = await chromium.launch();
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push(e.message));

    /* Serve repo files from disk; synthesize the manifest. External CDNs (cdn.edgepython.com) pass through. */
    await page.route("**/*", (route) => {
        const url = new URL(route.request().url());
        // In-tree runtime first; CI must test the checkout, not the deploy.
        if (url.host === CDN_HOST && url.pathname.startsWith("/runtime/")) {
            const path = RUNTIME + url.pathname.slice("/runtime/".length);
            try {
                return route.fulfill({ body: readFileSync(path), contentType: TYPES[path.slice(path.lastIndexOf("."))] ?? "application/octet-stream" });
            } catch {
                return route.continue();
            }
        }
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

    /* Per-case mocks; uninstalled after each case so they don't leak into later ones. */
    const httpMocks = [];
    const installMocks = async (c) => {
        for (const m of c.http_mocks ?? []) {
            const handler = (route) => route.fulfill({
                status: m.status ?? 200,
                contentType: m.contentType ?? "application/json",
                body: m.body ?? "",
            });
            await page.route(m.url, handler);
            httpMocks.push({ url: m.url, handler });
        }
        for (const m of c.ws_mocks ?? []) {
            await page.routeWebSocket(m.url, (ws) => {
                if (m.echo) ws.onMessage((message) => ws.send(message));
            });
        }
    };
    const uninstallMocks = async () => {
        for (const { url, handler } of httpMocks.splice(0)) await page.unroute(url, handler);
    };

    const failures = [];
    try {
        await page.goto("http://localhost/tests/index.html");
        // Boot the tag once without an entry, reuse its worker, and capture stdout via onOutput. It lives in <head> so the per-case body wipe leaves it connected, and so dom cases counting body children never see the tag.
        await page.evaluate(async (manifestPath) => {
            const el = document.createElement("edge-python");
            el.setAttribute("packages", manifestPath);
            const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
            document.head.appendChild(el);
            await ready;
            // Byte-stream stdout: one chunk per print() call (body + its `end`); collect verbatim.
            globalThis.chunks = [];
            el.worker.onOutput((chunk) => { globalThis.chunks.push(chunk); });
            // DBs present once the runtime is up (its integrity cache); resetState must leave these alone.
            globalThis.baseline = indexedDB.databases ? (await indexedDB.databases()).map((d) => d.name) : [];
            globalThis.el = el;
        }, MANIFEST);

        for (const [i, c] of cases.entries()) {
            await installMocks(c);
            const src = `from ${cap} import *\n${c.src}`;
            const result = await page.evaluate(async ({ s, html }) => {
                document.body.innerHTML = html ?? "";
                localStorage.clear();
                sessionStorage.clear();
                if (indexedDB.databases) {
                    const dbs = await indexedDB.databases();
                    await Promise.all(dbs.filter(({ name }) => name && !globalThis.baseline.includes(name)).map(({ name }) => new Promise((res) => {
                        const req = indexedDB.deleteDatabase(name);
                        req.onsuccess = req.onerror = req.onblocked = () => res();
                    })));
                }
                globalThis.chunks = [];
                const { out } = await globalThis.el.worker.run(s);
                // One entry per print() call; drop its single trailing newline (the `end`).
                const output = globalThis.chunks.map((c) => c.replace(/\n$/, ""));
                return { output, error: out || null };
            }, { s: src, html: c.html });
            await uninstallMocks();

            if (c.error) {
                if (!result.error || !result.error.includes(c.error)) {
                    failures.push(`[${cap} #${i}] expected error containing '${c.error}', got: ${result.error ?? "(none)"}`);
                }
                continue;
            }
            if (result.error) {
                failures.push(`[${cap} #${i}] unexpected error: ${result.error}`);
                continue;
            }
            const expected = c.output ?? [];
            if (JSON.stringify(result.output) !== JSON.stringify(expected)) {
                failures.push(`[${cap} #${i}] output mismatch\n  src: ${c.src.replaceAll("\n", " / ")}\n  expected: ${JSON.stringify(expected)}\n  got: ${JSON.stringify(result.output)}`);
            }
        }

        if (errors.length) failures.push(`[${cap}] console errors: ${errors.join(" | ")}`);
    } finally {
        await browser.close();
    }

    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

for (const cap of capabilities) {
    Deno.test(`host capability: ${cap}`, () => runCapability(cap));
}
