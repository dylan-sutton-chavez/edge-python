import { createWorker } from "./index.js";

// Bump together with the CLI harness.
globalThis.__edgeContract = "0.1.0";

/* Defines the custom element, a web component that allows importing the runtime using an HTML tag. https://developer.mozilla.org/en-US/docs/Web/API/Web_components */
export class EdgePythonElement extends HTMLElement { 
    async connectedCallback() {
        const file = this.getAttribute('entry');
        const pkg = this.getAttribute('packages');

// system -> main-thread modules (lazy, name -> url, imported on first use), imports -> worker .py/.wasm modules
        const systemModules = {};
        let imports;
        if (pkg) {
            const base = new URL(pkg, location.href);
            const manifest = await fetch(base).then(r => r.json());
            for (const [name, url] of Object.entries(manifest.system ?? {})) {
                systemModules[name] = new URL(url, base).href;
            }
            if (manifest.imports) {
                imports = {};
                for (const [name, url] of Object.entries(manifest.imports)) imports[name] = new URL(url, base).href;
            }
        }

        // Kept on the element so callers can drive the same worker after the declarative run.
        this.worker = await createWorker({
            wasmUrl: this.getAttribute("wasm") ?? "https://cdn.edgepython.com/compiler.wasm",
            systemModules,
            imports,
        });
        // `entry` is optional, omit it to just spin up the worker and drive it via run().
        if (file) await this.worker.run(await fetch(file).then(r => r.text()));
        this.dispatchEvent(new Event("ready"));
    }
}

export function defineElement( tag = 'edge-python' ) {
    customElements.define(tag, EdgePythonElement);
}

// In some environments (e.g. deno, node) pass `?setElement=false` to skip auto-defining the element, since `customElements` doesn't exist there.
const setElement = new URL(import.meta.url).searchParams.get("setElement");
if (setElement != "false") defineElement();
