// deno-lint-ignore no-import-prefix
import { chromium } from "npm:playwright@latest";
import { readFileSync } from "node:fs";
import { DEFAULT_IMPORTS } from "../src/defaults.ts";

// One CDN host now serves every family under a path prefix (/std, /system, /runtime), derive it from the manifest.
const CDN_HOST = new URL(Object.values(DEFAULT_IMPORTS)[0]).host;

const REPO = new URL("../../", import.meta.url).pathname; // edge-python/ repo root
const cases = JSON.parse(readFileSync(new URL("./web.json", import.meta.url)));
const PKG = JSON.parse(readFileSync(new URL("./app/packages.json", import.meta.url)));
// star-import every module key, recursing through the imports/system category containers
const star = (m) => Object.entries(m).flatMap(([k, v]) => (k === "imports" || k === "system" ? star(v) : `from ${k} import *`));
const PRELUDE = star(PKG).join("\n") + "\n";
const TYPES = {
    ".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html",
    ".py": "text/x-python", ".json": "application/json",
};

// Build web/src TypeScript into web/dist so the browser and the bootstrap test can load it.
let distBuilt = false;
async function buildDist() {
    if (distBuilt) return;
    const tsc = (cfg) => new Deno.Command(Deno.execPath(), { args: ["run", "-A", "npm:typescript@5.9.3/tsc", "-p", cfg], cwd: new URL("../", import.meta.url).pathname }).output();
    for (const c of ["tsconfig.json", "tsconfig.worker.json"]) { const r = await tsc(c); if (!r.success) throw new Error(`tsc: ${c}`); }
    distBuilt = true;
}

/* Minimal wasm-pdk module built by hand, `__edge_abi_version` reports `abi` and `boom` traps when called. */
function pdkModule(abi) {
    const enc = new TextEncoder();
    const leb = (n) => { const out = []; do { let b = n & 0x7f; n >>>= 7; if (n !== 0) b |= 0x80; out.push(b); } while (n !== 0); return out; };
    const vec = (items) => [...leb(items.length), ...items.flat()];
    const name = (s) => vec([...enc.encode(s)].map((b) => [b]));
    const section = (id, body) => [id, ...leb(body.length), ...body];
    const body = (code) => { const b = [0x00, ...code, 0x0b]; return [...leb(b.length), ...b]; };
    const types = section(1, vec([[0x60, 0x00, 0x01, 0x7f], [0x60, 0x01, 0x7f, 0x01, 0x7f], [0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x01, 0x7f]]));
    const funcs = section(3, vec([[0], [1], [2]]));
    const memory = section(5, vec([[0x00, 0x01]]));
    const exports = section(7, vec([
        [...name("memory"), 0x02, 0x00],
        [...name("__edge_abi_version"), 0x00, 0x00],
        [...name("__edge_alloc"), 0x00, 0x01],
        [...name("boom"), 0x00, 0x02],
    ]));
    const code = section(10, vec([body([0x41, ...leb(abi)]), body([0x41, 0x00]), body([0x00])]));
    return new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, ...types, ...funcs, ...memory, ...exports, ...code]);
}

/* Drives <edge-python> through index.html, boots one tag, then feeds every web.json case to its worker via run(), comparing #app for output cases and the run trace for error cases. Run with deno test --allow-all web/tests/web.test.js. */
Deno.test("runtime: <edge-python> runs the corpus through index.html", async () => {
    await buildDist();

    const browser = await chromium.launch();
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    const requested = [];
    page.on("request", (q) => requested.push(q.url()));

    const STD_DIR = new URL("../../std", import.meta.url).pathname;
    const SYSTEM_DIR = new URL("../../web/builtins", import.meta.url).pathname;
    await page.route("**/*", (r) => {
        const u = new URL(r.request().url());
        // Prefer the in-tree std/ and system/ artifacts, if absent (CI checks out only the js/ subset) fall back to the CDN-deployed copy.
        if (u.host === CDN_HOST && u.pathname.startsWith("/std/")) {
            // /std/<name>.wasm lives at <name>/target/wasm32-unknown-unknown/release/ in the tree.
            const name = u.pathname.slice("/std/".length).replace(/\.wasm$/, "");
            const file = `${STD_DIR}/${name}/target/wasm32-unknown-unknown/release/${name}.wasm`;
            try { return r.fulfill({ contentType: "application/wasm", body: readFileSync(file) }); }
            catch { return r.continue(); } // no local std build, use the deployed wasm
        }
        if (u.host === CDN_HOST && u.pathname.startsWith("/web/builtins/")) {
            // Production (Pages) flattens builtins/<cap>/src/* to builtins/<cap>/*, map back to the tree layout.
            const repoPath = u.pathname.replace(/^\/web\/builtins\/([^/]+)\//, "/$1/src/");
            try { return r.fulfill({ contentType: "text/javascript", body: readFileSync(SYSTEM_DIR + repoPath) }); }
            catch { return r.continue(); } // no local system source, use the deployed module
        }
        // Prefer in-tree wasm so new exports are testable.
        if (u.host === CDN_HOST && u.pathname === "/compiler.wasm") {
            const local = `${REPO}target/wasm32-unknown-unknown/release/compiler.wasm`;
            try { return r.fulfill({ contentType: "application/wasm", body: readFileSync(local) }); }
            catch { return r.continue(); } // no local build, use the deployed wasm
        }
        if (u.host !== "localhost") return r.continue(); // any other CDN asset (compiler.wasm, runtime) passes through
        if (u.pathname.endsWith("/app/trap.wasm")) return r.fulfill({ contentType: "application/wasm", body: pdkModule(1) });
        if (u.pathname.endsWith("/app/abi2.wasm")) return r.fulfill({ contentType: "application/wasm", body: pdkModule(2) });
        const ext = u.pathname.slice(u.pathname.lastIndexOf("."));
        try { return r.fulfill({ contentType: TYPES[ext] ?? "application/octet-stream", body: readFileSync(REPO + u.pathname.slice(1)) }); }
        catch { return r.fulfill({ status: 404 }); }
    });
    // A mock WebSocket echo server so network cases can open, echo and close sockets without leaving the page.
    await page.routeWebSocket("wss://localhost/echo", (ws) => {
        ws.onMessage((m) => ws.send(m));
    });
    await page.goto("http://localhost/web/tests/index.html");

    try {
        // Boot one tag without an entry, then reuse its worker for every case via run().
        await page.evaluate(async () => {
            const el = document.createElement("edge-python");
            el.setAttribute("packages", "./app/packages.json");
            const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
            document.body.appendChild(el);
            await ready;
            globalThis.el = el;
        });

        const reqd = (frag) => requested.some((u) => u.includes(frag));
        // Lazy system, a system ESM must not load at boot, only when a run first imports it.
        if (reqd("/app/ui.js")) throw new Error("system ui.js loaded at boot; system modules must be lazy");

        for (const c of cases) {
            errors.length = 0;
            const got = await page.evaluate(async (src) => {
                const app = document.querySelector("#app");
                app.textContent = "";
                const { out } = await globalThis.el.worker.run(src);
                return { app: app.textContent, out };
            }, PRELUDE + c.script);

            if (c.error) {
                if (!got.out.includes(c.error)) {
                    throw new Error(`script:\n${c.script}\n  want error containing: ${JSON.stringify(c.error)}\n  got out: ${JSON.stringify(got.out)}\n  errors: ${errors.join(" | ") || "(none)"}`);
                }
            } else if (got.app !== c.expect) {
                throw new Error(`script:\n${c.script}\n  got:  ${JSON.stringify(got.app)}\n  want: ${JSON.stringify(c.expect)}\n  out: ${JSON.stringify(got.out)}\n  errors: ${errors.join(" | ") || "(none)"}`);
            }
        }

        // Park on receive(), save, finish, restore, steer differently.
        const snap = await page.evaluate(async () => {
            const el = globalThis.el;
            const chunks = [];
            el.worker.onOutput((c) => chunks.push(c));
            const src = "history = []\nwhile True:\n    m = receive()\n    if m == 'stop':\n        break\n    history.append(m)\nprint('|'.join(history))";
            const running = el.worker.run(src);
            const parked = async () => {
                for (let i = 0; i < 100; i++) {
                    if (JSON.stringify(await el.worker.stateStack()).includes("waiting_event")) return;
                    await new Promise((r) => setTimeout(r, 20));
                }
                throw new Error("run never parked on receive()");
            };
            await parked();
            el.worker.pushEvent("a");
            await parked();
            const blob = await el.worker.saveState();
            const globalsAtSave = await el.worker.stateGlobals();
            el.worker.pushEvent("b");
            el.worker.pushEvent("stop");
            await running;
            const first = chunks.join("");
            chunks.length = 0;
            const resumed = el.worker.restoreState(blob);
            el.worker.pushEvent("c");
            el.worker.pushEvent("d");
            el.worker.pushEvent("stop");
            await resumed;
            return { first, second: chunks.join(""), globalsAtSave, blobLen: blob.length };
        });
        if (snap.first !== "a|b\n") throw new Error(`snapshot: original run produced ${JSON.stringify(snap.first)}`);
        if (snap.second !== "a|c|d\n") throw new Error(`snapshot: restored run produced ${JSON.stringify(snap.second)}`);
        if (snap.globalsAtSave.history !== "['a']") throw new Error(`snapshot: stateGlobals saw ${JSON.stringify(snap.globalsAtSave)}`);
        if (!(snap.blobLen > 100)) throw new Error(`snapshot: implausible blob length ${snap.blobLen}`);

        // A suspension-free program still pauses and snapshots.
        const pre = await page.evaluate(async () => {
            const el = globalThis.el;
            const chunks = [];
            el.worker.onOutput((c) => chunks.push(c));
            await el.worker.setPreemptInterval(50000);
            const src = "n = 0\nwhile n < 1000000:\n    n = n + 1\nprint('done', n)";
            const running = el.worker.run(src);
            await el.worker.pause();
            const globalsAtPause = await el.worker.stateGlobals();
            const blob = await el.worker.saveState();
            el.worker.resume();
            await running;
            const first = chunks.join("");
            chunks.length = 0;
            await el.worker.restoreState(blob);
            await el.worker.setPreemptInterval(0);
            return { first, second: chunks.join(""), globalsAtPause, blobLen: blob.length };
        });
        if (pre.first !== "done 1000000\n") throw new Error(`preempt: original run produced ${JSON.stringify(pre.first)}`);
        if (pre.second !== "done 1000000\n") throw new Error(`preempt: restored run produced ${JSON.stringify(pre.second)}`);
        const pausedAt = Number(pre.globalsAtPause.n);
        if (!(pausedAt > 0 && pausedAt < 1000000)) throw new Error(`preempt: expected a mid-loop pause, n was ${JSON.stringify(pre.globalsAtPause.n)}`);
        if (!(pre.blobLen > 100)) throw new Error(`preempt: implausible blob length ${pre.blobLen}`);

        // A pause on an event yield holds the program until resume().
        const evPause = await page.evaluate(async () => {
            const el = globalThis.el;
            const chunks = [];
            el.worker.onOutput((c) => chunks.push(c));
            const running = el.worker.run("m = receive()\nn = receive()\nprint(m, n)");
            let sawPark = false;
            for (let i = 0; i < 100; i++) {
                if (JSON.stringify(await el.worker.stateStack()).includes("waiting_event")) { sawPark = true; break; }
                await new Promise((r) => setTimeout(r, 20));
            }
            if (!sawPark) throw new Error("run never parked on receive()");
            const parked = el.worker.pause();
            el.worker.pushEvent("a");
            if (!await parked) throw new Error("pause() did not park an event-parked run");
            el.worker.pushEvent("b");
            await new Promise((r) => setTimeout(r, 200));
            const held = chunks.join("");
            el.worker.resume();
            await running;
            return { held, out: chunks.join("") };
        });
        if (evPause.held !== "") throw new Error(`pause: event-parked run kept running after pause(), saw ${JSON.stringify(evPause.held)}`);
        if (evPause.out !== "a b\n") throw new Error(`pause: after resume expected 'a b\\n', got ${JSON.stringify(evPause.out)}`);

        // A pause requested during a sleep parks the run once the timer fires.
        const tmPause = await page.evaluate(async () => {
            const el = globalThis.el;
            const chunks = [];
            el.worker.onOutput((c) => chunks.push(c));
            const running = el.worker.run("import time\nprint('start')\ntime.sleep(0.5)\nprint('end')");
            let sawSleep = false;
            for (let i = 0; i < 100; i++) {
                if (chunks.join("").includes("start")) { sawSleep = true; break; }
                await new Promise((r) => setTimeout(r, 20));
            }
            if (!sawSleep) throw new Error("run never reached sleep()");
            const parked = await el.worker.pause();
            await new Promise((r) => setTimeout(r, 300));
            const held = chunks.join("");
            el.worker.resume();
            await running;
            return { parked, held, out: chunks.join("") };
        });
        if (tmPause.parked !== true) throw new Error("pause: sleep-parked run did not report parked");
        if (tmPause.held !== "start\n") throw new Error(`pause: timer-parked run kept running after pause(), saw ${JSON.stringify(tmPause.held)}`);
        if (tmPause.out !== "start\nend\n") throw new Error(`pause: after resume expected 'start\\nend\\n', got ${JSON.stringify(tmPause.out)}`);

        // Documented tag path, fresh element via proxy.
        const tagged = await page.evaluate(async () => {
            const el = document.createElement("edge-python");
            const ready = new Promise((res) => el.addEventListener("ready", res, { once: true }));
            document.body.appendChild(el);
            await ready;
            await el.worker.setPreemptInterval(50000);
            const running = el.worker.run("n = 0\nwhile n < 1000000:\n    n = n + 1\nprint('done', n)");
            await el.worker.pause();
            const blobLen = (await el.worker.saveState()).length;
            el.worker.resume();
            const { out } = await running;
            el.worker.dispose();
            return { blobLen, out };
        });
        if (!(tagged.blobLen > 100)) throw new Error(`preempt via element: implausible blob length ${tagged.blobLen}`);
        if (tagged.out !== "") throw new Error(`preempt via element: run reported ${JSON.stringify(tagged.out)}`);

        // Laziness, only what the corpus imports gets fetched. Declared-but-unused stays untouched.
        if (!reqd("/app/ui.js")) throw new Error("system ui was used but ui.js never loaded");
        if (!reqd("json.wasm")) throw new Error("json default imported but json.wasm never fetched");
        if (!reqd("/web/builtins/time")) throw new Error("time system default imported but never loaded");
        if (reqd("re.wasm")) throw new Error("re default never imported yet re.wasm was fetched (not lazy)");
        if (!reqd("/web/builtins/network")) throw new Error("network imported by the ws cases but never loaded");

        // The IndexedDB cache survives a versionless boot and is wiped only by a version mismatch.
        const idb = await page.evaluate(async () => {
            if (!globalThis.el.worker.integrityActive) return null;
            const readStore = () => new Promise((res, rej) => {
                const req = indexedDB.open("edgepython", 1);
                req.onsuccess = () => {
                    const db = req.result;
                    const tx = db.transaction("lockfile");
                    const store = tx.objectStore("lockfile");
                    const out = { count: 0, version: null };
                    store.count().onsuccess = (e) => { out.count = e.target.result; };
                    store.get("\0v").onsuccess = (e) => { out.version = e.target.result ?? null; };
                    tx.oncomplete = () => { db.close(); res(out); };
                    tx.onerror = () => rej(tx.error);
                };
                req.onerror = () => rej(req.error);
            });
            const { createWorker } = await import("/web/dist/index.js");
            const spawn = (opts) => createWorker({ wasmUrl: "https://cdn.edgepython.com/compiler.wasm", ...opts });
            const before = await readStore();
            const plain = await spawn();
            const afterPlain = await readStore();
            plain.dispose();
            const v1 = await spawn({ version: "t-v1" });
            const afterV1 = await readStore();
            v1.dispose();
            const v1again = await spawn({ version: "t-v1" });
            const afterV1again = await readStore();
            v1again.dispose();
            const v2 = await spawn({ version: "t-v2" });
            const afterV2 = await readStore();
            v2.dispose();
            return { before, afterPlain, afterV1, afterV1again, afterV2 };
        });
        if (idb) {
            if (!(idb.before.count > 0)) throw new Error(`cache: corpus left an empty lockfile store ${JSON.stringify(idb.before)}`);
            if (idb.afterPlain.count !== idb.before.count) throw new Error(`cache: versionless boot wiped the cache ${JSON.stringify(idb)}`);
            if (idb.afterV1.count !== 1 || idb.afterV1.version !== "t-v1") throw new Error(`cache: fresh version should wipe then stamp ${JSON.stringify(idb.afterV1)}`);
            if (idb.afterV1again.count !== 1 || idb.afterV1again.version !== "t-v1") throw new Error(`cache: matching version wiped the cache ${JSON.stringify(idb.afterV1again)}`);
            if (idb.afterV2.count !== 1 || idb.afterV2.version !== "t-v2") throw new Error(`cache: version mismatch should wipe then restamp ${JSON.stringify(idb.afterV2)}`);
        }
    } finally {
        await browser.close();
    }
});

// The blob bootstrap posts a requestless error when the cross-origin import fails, createWorker must reject with it instead of hanging.
Deno.test("runtime: createWorker rejects on a worker bootstrap failure", async () => {
    await buildDist();
    const { createWorker } = await import("../dist/index.js");
    const RealWorker = globalThis.Worker;
    const hadLocation = "location" in globalThis;
    const RealLocation = globalThis.location;
    class StubWorker {
        constructor() {
            this.onmessage = null;
            this.onerror = null;
        }
        postMessage(msg) {
            if (msg.type === "load") {
                queueMicrotask(() => this.onmessage?.({ data: { type: "error", message: "worker bootstrap failed: boom" } }));
            }
        }
        terminate() {}
    }
    Object.defineProperty(globalThis, "Worker", { value: StubWorker, configurable: true, writable: true });
    // A page origin different from the module origin forces the Blob bootstrap path.
    globalThis.location = new URL("http://localhost/");
    try {
        let timer;
        const result = await Promise.race([
            createWorker().then(() => null, (e) => e),
            new Promise((_, rej) => { timer = setTimeout(() => rej(new Error("createWorker hung on bootstrap failure")), 5000); }),
        ]);
        clearTimeout(timer);
        if (!(result instanceof Error)) throw new Error("createWorker resolved despite the bootstrap failure");
        if (!String(result.message).includes("worker bootstrap failed: boom")) {
            throw new Error("unexpected rejection: " + result.message);
        }
        // Let the bootstrap's deferred revokeObjectURL timer fire.
        await new Promise((r) => setTimeout(r, 10));
    } finally {
        Object.defineProperty(globalThis, "Worker", { value: RealWorker, configurable: true, writable: true });
        if (hadLocation) globalThis.location = RealLocation;
        else delete globalThis.location;
    }
});

