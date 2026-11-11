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

/** `response` with its body counted as it streams, reporting bytes read against `total`. */
function counted(response, total, onProgress) {
  if (!response.body || !onProgress) return response;
  const reader = response.body.getReader();
  let loaded = 0;
  const body = new ReadableStream({
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) {
        controller.close();
        return;
      }
      loaded += value.byteLength;
      onProgress(loaded, total);
      controller.enqueue(value);
    },
    cancel: (reason) => reader.cancel(reason),
  });
  return new Response(body, { headers: { "content-type": "application/wasm" } });
}

/**
 * Fetches and compiles each backend's module once, compiling while it downloads, and loads its
 * bindings script; a backend can be started early with `prefetch` while the page probes.
 */
export function networkLoader({ manifest, log, onProgress }) {
  const modules = new Map();
  const scripts = new Map();

  function compile(backend) {
    if (!modules.has(backend)) {
      const bundle = manifest[backend];
      const compiling = (async () => {
        log("fetch", { backend, bytes: bundle.size });
        let response;
        try {
          response = await fetch(`assets/${bundle.wasm}`);
          if (!response.ok) throw new Error(`the server answered ${response.status} for ${bundle.wasm}`);
        } catch (error) {
          throw failure("fetch", backend, error);
        }
        const body = counted(response, bundle.size, onProgress && ((loaded, total) => onProgress(backend, loaded, total)));
        try {
          const module = WebAssembly.compileStreaming
            ? await WebAssembly.compileStreaming(body)
            : await WebAssembly.compile(await body.arrayBuffer());
          log("compiled", { backend });
          return module;
        } catch (error) {
          throw failure("compile", backend, error);
        }
      })();
      // A failed download is tried afresh on the next attempt rather than remembered.
      compiling.catch(() => modules.delete(backend));
      modules.set(backend, compiling);
    }
    return modules.get(backend);
  }

  function bindings(backend) {
    if (!scripts.has(backend)) {
      const loading = script(`assets/${manifest[backend].js}`).catch((error) => {
        scripts.delete(backend);
        throw failure("fetch", backend, error);
      });
      scripts.set(backend, loading);
    }
    return scripts.get(backend);
  }

  return {
    prefetch(backend) {
      compile(backend).catch(() => {});
      bindings(backend).catch(() => {});
    },
    async load(backend) {
      const [module] = await Promise.all([compile(backend), bindings(backend)]);
      return { factory: globalThis.currentsBundles[backend], module };
    },
  };
}
