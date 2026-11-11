import { failure } from "../core/failure.js";

/** Adds `src` as a classic script and waits for it to run. */
function script(src) {
  return new Promise((resolve, reject) => {
    const element = document.createElement("script");
    element.src = src;
    element.onload = resolve;
    element.onerror = () => reject(new Error(`could not load ${src}`));
    document.head.append(element);
  });
}

/** Base64 to bytes with the browser's own decoder rather than a loop in script. */
async function decode(text) {
  if (Uint8Array.fromBase64) return Uint8Array.fromBase64(text);
  const response = await fetch(`data:application/octet-stream;base64,${text}`);
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * Loads each backend from the package beside the page. A page a host opens from disk may not
 * fetch files, so each module comes inside a script as base64; it is compiled once and kept,
 * since a wallpaper may start the same backend again after losing its device.
 */
export function packageLoader({ manifest, log }) {
  const modules = new Map();

  function compile(backend) {
    if (!modules.has(backend)) {
      const bundle = manifest[backend];
      const compiling = (async () => {
        try {
          await Promise.all([script(`assets/${bundle.js}`), script(`assets/${bundle.data}`)]);
        } catch (error) {
          throw failure("fetch", backend, error);
        }
        const text = globalThis.currentsWasm[backend];
        delete globalThis.currentsWasm[backend];
        try {
          const module = await WebAssembly.compile(await decode(text));
          log("compiled", { backend });
          return module;
        } catch (error) {
          throw failure("compile", backend, error);
        }
      })();
      compiling.catch(() => modules.delete(backend));
      modules.set(backend, compiling);
    }
    return modules.get(backend);
  }

  return {
    prefetch(backend) {
      compile(backend).catch(() => {});
    },
    async load(backend) {
      const module = await compile(backend);
      return { factory: globalThis.currentsBundles[backend], module };
    },
  };
}
