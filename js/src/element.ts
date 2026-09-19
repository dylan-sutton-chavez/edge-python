import { createWorker } from "./index.ts";
import type { WorkerHandle } from "./index.ts";

/* Defines the custom element, a web component that loads the JS host from an HTML tag. */
export class EdgePythonElement extends HTMLElement {
    worker?: WorkerHandle

    async connectedCallback() {
        const file = this.getAttribute('entry');
        const manifestUrl = this.getAttribute('manifest');

        // Each entry resolves against the manifest url, the artifact behind it decides where it runs.
        let imports: Record<string, string> | undefined;
        if (manifestUrl) {
            const base = new URL(manifestUrl, location.href);
            const manifest: { imports?: Record<string, string>, system?: unknown } = await fetch(base).then(r => r.json());
            if (manifest.system !== undefined) throw new Error(`edge.json at '${base.href}': move the system entries into imports`);
            if (manifest.imports) {
                imports = {};
                for (const [name, url] of Object.entries(manifest.imports)) imports[name] = new URL(url, base).href;
            }
        }

        // Kept on the element so callers can drive the same worker after the declarative run.
        this.worker = await createWorker({
            wasmUrl: this.getAttribute("wasm") ?? "https://cdn.edgepython.com/compiler.wasm",
            imports,
        });
        // `entry` is optional, omit it to just spin up the worker and drive it via run().
        if (file) await this.worker.run(await fetch(file).then(r => r.text()));
        this.dispatchEvent(new Event("ready"));
    }
}

export function defineElement(tag = 'edge-python') {
    customElements.define(tag, EdgePythonElement);
}

// In some environments (e.g. deno, node) pass `?setElement=false` to skip auto-defining the element, since `customElements` doesn't exist there.
const setElement = new URL(import.meta.url).searchParams.get("setElement");
if (setElement != "false") defineElement();
