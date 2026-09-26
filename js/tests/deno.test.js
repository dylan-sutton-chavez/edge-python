import { hostCallError } from "../src/util.ts";

/* The engine under Deno with no browser, undeclared names fail and missing Web APIs name themselves. */
const BASE = Deno.env.get("EDGE_CDN_BASE")?.replace(/\/$/, "");
if (!BASE) throw new Error("set EDGE_CDN_BASE (npm run cdn:local in infra)");
const WASM = `${BASE}/compiler.wasm`;

// A fresh engine per test, the query string keeps the module state apart.
async function boot(name, builtins) {
    const engine = await import(new URL(`../src/worker/engine.ts?deno=${name}`, import.meta.url).href);
    const handlers = {};
    const labels = {};
    const pushEvent = (m) => engine.pushEvent(m);
    engine.setLoadSystemDelegate(async (url, label) => {
        const source = await import(url);
        const factory = source.default ?? source[label];
        const h = typeof factory === "function" ? factory({ pushEvent }) : factory;
        labels[url] = label;
        for (const [k, v] of Object.entries(h)) handlers[`${url}:${k}`] = v;
        return Object.keys(h);
    });
    engine.setHostCallDelegate(async (url, fn, args) => {
        const h = handlers[`${url}:${fn}`];
        if (!h) throw new Error(`no main-thread handler for '${labels[url]}.${fn}'`);
        try {
            return await h(...args);
        } catch (e) {
            throw new Error(hostCallError(labels[url], e));
        }
    });
    // Each builtin is declared the way edge.json would, by the url of its JavaScript module.
    const imports = Object.fromEntries(builtins.map((b) => [b, new URL(`../builtins/${b}/src/index.js`, import.meta.url).href]));
    await engine.load({ wasmUrl: WASM, integrity: false, imports });
    return engine;
}

// No manifest lives here, so bare names stay undeclared.
const baseUrl = new URL("./nomanifest/", import.meta.url).href;

Deno.test("deno: an undeclared name fails at compile time", async () => {
    const engine = await boot("undeclared", []);
    const { out } = await engine.run({ src: "import json\nprint(1)", baseUrl });
    if (!out.includes("module 'json' is not provided by this host and no edge.json declares it")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

// 010100101010 THIS AND THE NEXT TEST LOAD TIME AND DOM, RESTORE THEM ONCE EDGE-PYTHON-STD PUBLISHES THEM TO THE REGISTRY.
Deno.test.ignore("deno: a declared JavaScript module answers", async () => {
    const engine = await boot("time", ["time"]);
    const lines = [];
    const { out } = await engine.run({ src: "from time import tzname\nprint(tzname())", baseUrl }, (t) => lines.push(t));
    if (out !== "") throw new Error(`run failed ${JSON.stringify(out)}`);
    if (lines.join("").trim() === "") throw new Error("tzname printed nothing");
});

Deno.test.ignore("deno: a browser module loads and names the Web API it lacks", async () => {
    const engine = await boot("dom", ["dom"]);
    const { out } = await engine.run({ src: "import dom\ndom.body()", baseUrl });
    if (!out.includes("module 'dom' needs 'document', missing in this runtime")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

Deno.test("deno: send() names the actor scheduler it lacks", async () => {
    const engine = await boot("send", []);
    const lines = [];
    const missing = "send() needs an actor scheduler, missing in this runtime";
    const caught = await engine.run({ src: "try:\n    send('g', 'x')\nexcept RuntimeError as e:\n    print(e)", baseUrl }, (t) => lines.push(t));
    if (caught.out !== "" || lines.join("").trim() !== missing) throw new Error(`unexpected ${JSON.stringify([caught.out, lines])}`);
    const { out } = await engine.run({ src: "send('g', 'x')", baseUrl });
    if (!out.includes(missing) || !out.includes("<input>:1:1")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

Deno.test("deno: a leftover system section is refused", async () => {
    const dir = await Deno.makeTempDir();
    await Deno.writeTextFile(`${dir}/edge.json`, JSON.stringify({ system: { time: "./time.js" } }));
    const engine = await boot("legacy", []);
    const { out } = await engine.run({ src: "import time", baseUrl: `file://${dir}/` });
    await Deno.remove(dir, { recursive: true });
    if (!out.includes("edge.json at 'edge.json': move the system entries into imports")) throw new Error(`unexpected output ${JSON.stringify(out)}`);
});

Deno.test("deno: a manifest beside a module joins its relative targets once", async () => {
    const dir = await Deno.makeTempDir();
    await Deno.mkdir(`${dir}/pkg`);
    await Deno.writeTextFile(`${dir}/edge.json`, JSON.stringify({ imports: { pkg: "./pkg/entry.py" } }));
    await Deno.writeTextFile(`${dir}/pkg/edge.json`, JSON.stringify({ imports: { _impl: "./impl.py" } }));
    await Deno.writeTextFile(`${dir}/pkg/entry.py`, "from _impl import value\n");
    await Deno.writeTextFile(`${dir}/pkg/impl.py`, "value = 42\n");
    const engine = await boot("facade", []);
    const lines = [];
    const { out } = await engine.run({ src: "from pkg import value\nprint(value)", baseUrl: `file://${dir}/` }, (t) => lines.push(t));
    await Deno.remove(dir, { recursive: true });
    if (out !== "" || lines.join("").trim() !== "42") throw new Error(`unexpected ${JSON.stringify([out, lines])}`);
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
