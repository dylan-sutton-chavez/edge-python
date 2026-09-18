import { hostCallError } from "../src/util.ts";

/* The engine under Deno with no browser, undeclared names fail and missing Web APIs name themselves. */
const WASM = new URL("../../target/wasm32-unknown-unknown/release/compiler.wasm", import.meta.url);
try {
    Deno.statSync(WASM);
} catch {
    throw new Error(`${WASM.pathname} is missing, run cargo wasm first`);
}

// A fresh engine per test, the query string keeps the module state apart.
async function boot(name, systems) {
    const engine = await import(new URL(`../src/worker/engine.ts?deno=${name}`, import.meta.url).href);
    const handlers = {};
    const pushEvent = (m) => engine.pushEvent(m);
    engine.setLoadSystemDelegate(async (mod) => {
        const source = await import(new URL(`../builtins/${mod}/src/index.js`, import.meta.url).href);
        const factory = source[mod] ?? source.default;
        const h = typeof factory === "function" ? factory({ pushEvent }) : factory;
        for (const [k, v] of Object.entries(h)) handlers[`${mod}:${k}`] = v;
        return Object.keys(h);
    });
    engine.setHostCallDelegate(async (mod, fn, args) => {
        const h = handlers[`${mod}:${fn}`];
        if (!h) throw new Error(`no main-thread handler for '${mod}.${fn}'`);
        try {
            return await h(...args);
        } catch (e) {
            throw new Error(hostCallError(mod, e));
        }
    });
    await engine.load({ wasmUrl: WASM.href, integrity: false, imports: {}, availableSystems: systems });
    return engine;
}

// No manifest lives here, so bare names stay undeclared.
const baseUrl = new URL("./nomanifest/", import.meta.url).href;

Deno.test("deno: an undeclared name fails at compile time", async () => {
    const engine = await boot("undeclared", []);
    const { out } = await engine.run({ src: "import json\nprint(1)", baseUrl });
    if (!out.includes("module 'json' is not provided by this host and no packages.json declares it")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

Deno.test("deno: a declared system module answers", async () => {
    const engine = await boot("time", ["time"]);
    const lines = [];
    const { out } = await engine.run({ src: "from time import tzname\nprint(tzname())", baseUrl }, (t) => lines.push(t));
    if (out !== "") throw new Error(`run failed ${JSON.stringify(out)}`);
    if (lines.join("").trim() === "") throw new Error("tzname printed nothing");
});

Deno.test("deno: a browser module loads and names the Web API it lacks", async () => {
    const engine = await boot("dom", ["dom"]);
    const { out } = await engine.run({ src: "import dom\ndom.body()", baseUrl });
    if (!out.includes("module 'dom' needs 'document', missing in this runtime")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

Deno.test("deno: frame() names the Web API it lacks", async () => {
    const engine = await boot("frame", []);
    let message = "";
    try {
        await engine.run({ src: "frame()\nprint('after')", baseUrl });
    } catch (e) {
        message = e.message;
    }
    if (message !== "frame() needs requestAnimationFrame, missing in this runtime") throw new Error(`unexpected rejection ${JSON.stringify(message)}`);
});
