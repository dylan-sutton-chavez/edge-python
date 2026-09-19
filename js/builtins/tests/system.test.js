// deno-lint-ignore no-import-prefix
import { chromium } from "npm:playwright@latest";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { Buffer } from "node:buffer";

const ROOT = new URL("../", import.meta.url).pathname;
const CORPUS = new URL("../../../tests/cases/builtins/", import.meta.url).pathname;
const CDN_HOST = "cdn.edgepython.com";
// The staged CDN this run tests, a tmp prefix in CI or the local one from infra.
const BASE = Deno.env.get("EDGE_CDN_BASE")?.replace(/\/$/, "");
if (!BASE) throw new Error("set EDGE_CDN_BASE (npm run cdn:local in infra)");
const MANIFEST = "/_edge.json"; // synthesized, keeps the agnostic <cap>/ folder free of test artifacts

// Cases per capability, the shared corpus under CORPUS first, then the browser-only one beside the module.
const loadCases = (cap) => [`${CORPUS}${cap}.json`, `${ROOT}${cap}/${cap}.json`]
    .filter((path) => existsSync(path))
    .flatMap((path) => JSON.parse(readFileSync(path, "utf-8")));

/* A capability is any dir with a corpus or a shared one in tests/cases/builtins, SYSPKG picks one. */
const only = Deno.env.get("SYSPKG");
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

/* The official origin answers from BASE, decoded bytes and the CDN's own headers, CORS included. */
async function cdn(route, url) {
    const res = await fetch(BASE + url.pathname + url.search);
    const headers = Object.fromEntries([...res.headers].filter(([k]) => k !== "content-encoding" && k !== "content-length"));
    return route.fulfill({ status: res.status, headers, body: Buffer.from(await res.arrayBuffer()) });
}

// Boots the network fixture, its port fills the corpus placeholders.
async function startMock() {
    const script = new URL("./mock.ts", import.meta.url).pathname;
    const child = new Deno.Command(Deno.execPath(), { args: ["run", "--allow-net", script], stdout: "piped" }).spawn();
    // The fixture prints its port on the first stdout line.
    const reader = child.stdout.getReader();
    const line = new TextDecoder().decode((await reader.read()).value);
    await reader.cancel(); // the port line is all the fixture ever prints
    return { child, port: parseInt(line, 10) };
}

async function runCapability(cap) {
    const dir = `${ROOT}${cap}`;
    // Import the capability's `.py` entry when it has one, else its JavaScript module, both from the CDN.
    const hasPy = existsSync(`${dir}/src/entry.py`);

    const cases = loadCases(cap);
    // The tag's edge.json, pinned by the capability or synthesized around its published url.
    const manifest = existsSync(`${dir}/edge.json`)
        ? readFileSync(`${dir}/edge.json`, "utf-8")
        : JSON.stringify({ imports: { [cap]: `https://${CDN_HOST}/js/builtins/${cap}/${hasPy ? "entry.py" : "index.js"}` } });

    // Chromium's Local Network Access guard would block loopback, so the test browser disables that check.
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

    /* Serve the harness page and the synthesized manifest, the fixture host stays unrouted so sse flows. */
    const strays = new Set();
    const fixture = mock ? `127.0.0.1:${mock.port}` : null;
    await page.route((url) => url.host !== fixture, (route) => {
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
        await page.goto("http://localhost/tests/index.html");
        // Booted once in <head>, the per-case body wipe keeps it and dom cases never count it.
        let bootTimer;
        try {
            await Promise.race([
                page.evaluate(async (manifestPath) => {
                    const el = document.createElement("edge-python");
                    el.setAttribute("manifest", manifestPath);
                    const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
                    document.head.appendChild(el);
                    await ready;
                    // Byte-stream stdout, one chunk per print() call (body + its `end`). Collect verbatim.
                    globalThis.chunks = [];
                    el.worker.onOutput((chunk) => { globalThis.chunks.push(chunk); });
                    // DBs present once the JS host is up (its integrity cache), resetState must leave these alone.
                    globalThis.baseline = indexedDB.databases ? (await indexedDB.databases()).map((d) => d.name) : [];
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
                // One entry per print() call, drop its single trailing newline (the `end`).
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
    } catch (e) {
        // A stray request explains any failure it caused, report it first.
        throw strays.size ? new Error([...strays].join("\n"), { cause: e }) : e;
    } finally {
        await browser.close();
        if (mock) {
            mock.child.kill();
            await mock.child.status;
        }
    }

    if (strays.size) failures.unshift(...strays);
    if (failures.length) throw new Error("\n" + failures.join("\n"));
}

/* Feeds each capability corpus to the <edge-python> tag, browser-only corpora sit beside the module. */
for (const cap of capabilities) {
    Deno.test(`system package: ${cap}`, () => runCapability(cap));
}
