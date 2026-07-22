/**
 * Define the custom element, which is a web component that allows to import the runtime using an HTML tag.
 * https://developer.mozilla.org/en-US/docs/Web/API/Web_components
 */

import { createWorker } from "./index.js";

export class EdgePythonElement extends HTMLElement { 
    async connectedCallback() {
        const file = this.getAttribute('entry');
        const pkg = this.getAttribute('packages');

        // host -> main-thread modules (lazy: name -> url, imported on first use), imports -> worker .py/.wasm modules
        const hostModules = {};
        let imports;
        if (pkg) {
            const base = new URL(pkg, location.href);
            const manifest = await fetch(base).then(r => r.json());
            for (const [name, url] of Object.entries(manifest.host ?? {})) {
                hostModules[name] = new URL(url, base).href;
            }
            if (manifest.imports) {
                imports = {};
                for (const [name, url] of Object.entries(manifest.imports)) imports[name] = new URL(url, base).href;
            }
        }

        // Kept on the element so callers can drive the same worker after the declarative run.
        this.worker = await createWorker({
            wasmUrl: "https://cdn.edgepython.com/compiler.wasm",
            hostModules,
            imports,
        });
        // `entry` is optional: omit it to just spin up the worker and drive it via run().
        if (file) await this.worker.run(await fetch(file).then(r => r.text()));
        this.dispatchEvent(new Event("ready"));
    }
}

export function defineElement( tag = 'edge-python' ) {
    customElements.define(tag, EdgePythonElement);
}

// In some environment (e.g., deno, node) use: `?setElement=false` to skip auto-defining the element, due to `customElements` doesn't exist in that environment.
const setElement = new URL(import.meta.url).searchParams.get("setElement");
if (setElement != "false") defineElement();
