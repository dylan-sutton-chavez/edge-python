import { chromium, firefox, webkit } from "npm:playwright@latest";

// Without SITE_BASE the suite serves the built Worker itself, the way the route corpus does.
const PORT = 8788;
const BASE = Deno.env.get("SITE_BASE") ?? `http://127.0.0.1:${PORT}`;

// Only the horizontal axis. Text wraps at engine-specific widths, so heights drift on their own.
const AXES = [0, 2];

// A box that shrinks to fit its label lands up to 10px apart across engines, a real break moves hundreds.
const TOLERANCE = 12;
const REPORTED = 6;

const VIEWPORTS = [
  { name: "phone", width: 390, height: 844 },
  { name: "desktop", width: 1280, height: 800 },
];

// The first engine is the reference the others are compared against.
const ENGINES = [
  { name: "chromium", type: chromium },
  { name: "firefox", type: firefox },
  { name: "webkit", type: webkit },
];

const screens = JSON.parse(await Deno.readTextFile(new URL("./screens.json", import.meta.url)));

async function serve() {
  if (Deno.env.get("SITE_BASE")) return null;

  const site = new URL("../", import.meta.url).pathname;
  const child = new Deno.Command("node", {
    args: ["node_modules/wrangler/bin/wrangler.js", "dev", "--ip", "127.0.0.1", "--port", String(PORT)],
    cwd: site,
    stdin: "null",
  }).spawn();

  for (let i = 0; i < 120; i++) {
    try {
      if ((await fetch(`${BASE}/api/health`)).ok) return child;
    } catch { /* still booting */ }
    await new Promise((done) => setTimeout(done, 500));
  }

  child.kill();
  throw new Error(`wrangler dev never answered on ${BASE}`);
}

// A dev server compiles a cold route on the first hit and can answer it empty.
async function open(page, url) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await page.goto(url, { waitUntil: "load" });
    } catch (error) {
      if (attempt === 2) throw error;
      await page.waitForTimeout(1000);
    }
  }
}

// Highlighting adds nodes and a transition moves them, so wait for the whole geometry to hold still.
async function settle(page) {
  for (let previous = "", still = 0, round = 0; still < 2 && round < 20; round++) {
    const now = JSON.stringify(await page.evaluate(measure));
    still = now === previous ? still + 1 : 0;
    previous = now;
    await page.waitForTimeout(250);
  }
}

// A screen can open something before measuring, its trigger only exists at the viewport that shows it.
async function act(page, screen) {
  if (!screen.click) return true;

  const target = page.locator(screen.click).first();
  if (!(await target.isVisible())) return false;

  await target.click();
  return true;
}

// Every visible box on the page, keyed by a DOM path the three engines agree on.
function measure() {
  const boxes = {};

  const walk = (el, path) => {
    const rect = el.getBoundingClientRect();
    // An inline box is placed by the text around it, so where it lands is the engine's business.
    if ((rect.width || rect.height) && getComputedStyle(el).display !== "inline") {
      boxes[path] = [Math.round(rect.x), Math.round(rect.y), Math.round(rect.width), Math.round(rect.height)];
    }
    let nth = 0;
    for (const child of el.children) walk(child, `${path}>${child.tagName.toLowerCase()}[${++nth}]`);
  };

  walk(document.body, "body");
  return boxes;
}

const taken = {};
const skipped = new Set();
const failures = [];

// Every screen at every viewport in every engine, one launch each.
async function collect() {
  for (const engine of ENGINES) {
    const browser = await engine.type.launch();

    for (const viewport of VIEWPORTS) {
      const context = await browser.newContext({ viewport: { width: viewport.width, height: viewport.height } });
      const page = await context.newPage();

      for (const screen of screens) {
        const response = await open(page, BASE + screen.path);
        const label = `${screen.name} ${viewport.name}`;

        if (!response || !response.ok()) {
          failures.push(`[${label}] ${engine.name}: status ${response ? response.status() : "no response"}`);
          continue;
        }

        await page.evaluate(() => document.fonts.ready);
        await settle(page);

        if (!(await act(page, screen))) {
          skipped.add(label);
          continue;
        }

        await settle(page);
        (taken[label] ??= {})[engine.name] = await page.evaluate(measure);
      }

      await context.close();
    }

    await browser.close();
  }
}

const server = await serve();

try {
  await collect();
} finally {
  server?.kill();
}

// One engine against the reference, worst drift first so the real break leads.
function drift(base, other) {
  const found = [];

  for (const [path, box] of Object.entries(base)) {
    const mirror = other[path];
    if (!mirror) {
      found.push({ path, delta: Infinity, why: "missing" });
      continue;
    }
    const delta = Math.max(...AXES.map((axis) => Math.abs(box[axis] - mirror[axis])));
    if (delta > TOLERANCE) found.push({ path, delta, why: `${delta}px` });
  }

  for (const path of Object.keys(other)) {
    if (!(path in base)) found.push({ path, delta: Infinity, why: "only here" });
  }

  // One box that moves drags every box under it, so report the ancestor and drop its subtree.
  const roots = [];
  for (const each of found.sort((a, b) => a.path.length - b.path.length)) {
    if (!roots.some((root) => each.path.startsWith(`${root.path}>`))) roots.push(each);
  }

  return { roots: roots.sort((a, b) => b.delta - a.delta), total: found.length };
}

for (const [label, byEngine] of Object.entries(taken)) {
  const base = byEngine[ENGINES[0].name];
  const problems = [];

  for (const engine of ENGINES.slice(1)) {
    if (!base || !byEngine[engine.name]) continue;
    const { roots, total } = drift(base, byEngine[engine.name]);
    for (const each of roots.slice(0, REPORTED)) problems.push(`${engine.name} ${each.path} ${each.why}`);
    if (roots.length > REPORTED) problems.push(`${engine.name} and ${roots.length - REPORTED} more roots`);
    if (roots.length) problems.push(`${engine.name} ${roots.length} roots, ${total} boxes in total`);
  }

  console.log(`${problems.length ? "FAIL" : "ok  "} ${label}`);
  failures.push(...problems.map((problem) => `[${label}] ${problem}`));
}

for (const label of skipped) console.log(`skip ${label}, its trigger is hidden here`);

if (failures.length) {
  console.error(failures.join("\n"));
  Deno.exit(1);
}
