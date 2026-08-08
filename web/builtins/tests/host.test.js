/* Agnostic driver, feeds each capability corpus to the <edge-python> tag. Web-only corpora sit beside the module, shared ones live in tests/cases/builtins. Run deno test --allow-all tests/ */

import { chromium } from "npm:playwright@latest";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { DEFAULT_IMPORTS } from "../../src/defaults.js";

const ROOT = new URL("../", import.meta.url).pathname;
const RUNTIME = new URL("../../", import.meta.url).pathname;
const CORPUS = new URL("../../../tests/cases/builtins/", import.meta.url).pathname;
const CDN_HOST = new URL(Object.values(DEFAULT_IMPORTS)[0]).host;
const MANIFEST = "/_packages.json"; // synthesized; keeps the agnostic <cap>/ folder free of test artifacts

// Corpus path per capability. Shared web-native corpora live under CORPUS, web-only ones sit beside the module.
const corpusPath = (cap) => existsSync(`${CORPUS}${cap}.json`) ? `${CORPUS}${cap}.json` : `${ROOT}${cap}/${cap}.json`;

/* A capability is any module dir with a corpus, plus any shared corpus under tests/cases/builtins. `HOSTCAP=<name>` narrows to one, used by the matrix-fanned CI to isolate per-shard work. */
const only = Deno.env.get("HOSTCAP");
const local = readdirSync(ROOT).filter((name) => {
    const dir = ROOT + name;
    return statSync(dir).isDirectory() && existsSync(`${dir}/${name}.json`);
});
const shared = readdirSync(CORPUS).filter((f) => f.endsWith(".json")).map((f) => f.slice(0, -5));
const capabilities = [...new Set([...local, ...shared])].filter((name) => !only || name === only);

const TYPES = {
    ".html": "text/html",
    ".js": "text/javascript",
    ".wasm": "application/wasm",
    ".json": "application/json",
    ".svg": "image/svg+xml",
    ".py": "text/plain",
    ".css": "text/css",
};

// Boots the network fixture shared with the native runner, its port fills the corpus placeholders.
async function startMock() {
    const bin = new URL(`../../../cli/target/debug/mock${Deno.build.os === "windows" ? ".exe" : ""}`, import.meta.url).pathname;
    if (!existsSync(bin)) {
        throw new Error(`need the mock server, build it first: cargo build --manifest-path cli/Cargo.toml --bin mock`);
    }
    const child = new Deno.Command(bin, { stdout: "piped" }).spawn();
    // The fixture prints its port on the first stdout line.
    const reader = child.stdout.getReader();
    const line = new TextDecoder().decode((await reader.read()).value);
    reader.releaseLock();
    return { child, port: parseInt(line, 10) };
}

async function runCapability(cap) {
    const dir = `${ROOT}${cap}`;
    // Import the capability's `.py` entry when it has one, else the JS host module.
    const hasPy = existsSync(`${dir}/src/entry.py`);

    const cases = JSON.parse(readFileSync(corpusPath(cap), "utf-8"));
    // The tag's packages.json: a capability may pin its own (e.g. python wrapper + host module pairs), else synthesized: python to entry.py as a code module; else the JS host module.
    const manifest = existsSync(`${dir}/packages.json`)
        ? readFileSync(`${dir}/packages.json`, "utf-8")
        : JSON.stringify(
            hasPy
                ? { imports: { [cap]: `/${cap}/src/entry.py` } }
                : { host: { [cap]: `/${cap}/src/index.js` } },
        );

    // The fixture serves from loopback, Chromium's Local Network Access guard would block the test page
    // reaching it, so disable that check for the test browser (never shipped, only the CI Chromium).
    const browser = await chromium.launch({ args: ["--disable-features=LocalNetworkAccessChecks,LocalNetworkAccessChecksWebSockets"] });
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push(e.message));

    // Network cases hit the fixture over loopback, its base and ws base replace the corpus placeholders.
    const needsMock = cases.some((c) => c.src.includes("{BASE}") || c.src.includes("{WS_BASE}"));
    const mock = needsMock ? await startMock() : null;
    const base = mock ? `http://127.0.0.1:${mock.port}` : "";
    const wsBase = base.replace("http://", "ws://");

    /* Serve repo files from disk; synthesize the manifest. The fixture host is left unrouted so its real
       responses, including sse keep-alive streams, reach the browser directly rather than buffered. */
    await page.route((url) => url.host === "localhost" || url.host === CDN_HOST, (route) => {
        const url = new URL(route.request().url());
        // In-tree runtime first, CI must test the checkout not the deploy.
        if (url.host === CDN_HOST && url.pathname.startsWith("/web/")) {
            const path = RUNTIME + url.pathname.slice("/web/".length);
            try {
                return route.fulfill({ body: readFileSync(path), contentType: TYPES[path.slice(path.lastIndexOf("."))] ?? "application/octet-stream" });
            } catch {
                return route.continue();
            }
        }
        if (url.host === CDN_HOST) return route.continue();
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
            const body = c.src.replaceAll("{BASE}", base).replaceAll("{WS_BASE}", wsBase);
            const src = `from ${cap} import *\n${body}`;
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
        if (mock) {
            mock.child.kill();
            await mock.child.status;
        }
    }

    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

for (const cap of capabilities) {
    Deno.test(`host capability: ${cap}`, () => runCapability(cap));
}
