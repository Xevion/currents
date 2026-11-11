import { failure } from "./failure.js";

/** How long a stopped viewer gets to release its device before its canvas is taken away. */
const RELEASE_MS = 250;

/**
 * Starts one backend on a fresh canvas. Resolves to a session once the first frame is done on
 * the device, or rejects with a failure from whichever step broke; either way nothing of a
 * failed attempt stays on the page.
 *
 * `load(backend)` resolves to `{factory, module}`: a function returning fresh wasm-bindgen
 * bindings, and the compiled module to instantiate them with. `mount(canvas)` puts the canvas
 * on the page. `options` go to the viewer's `start` beside the callbacks.
 */
export async function attempt({ backend, load, mount, options, watchdogMs, log }) {
  const canvas = document.createElement("canvas");
  canvas.className = "stage";
  canvas.tabIndex = 0;
  canvas.dataset.backend = backend;

  let state = "starting";
  let viewer = null;
  let watchdog = 0;
  let resolveReady;
  let rejectReady;
  let resolveEnded;
  const ready = new Promise((resolve, reject) => ((resolveReady = resolve), (rejectReady = reject)));
  const ended = new Promise((resolve) => (resolveEnded = resolve));
  const session = { backend, canvas, gpu: null, ended, stop: teardown };

  function fail(reason) {
    if (state === "starting") {
      state = "over";
      log("attempt-failed", { backend, stage: reason.stage, message: reason.message, detail: reason.detail });
      teardown();
      rejectReady(reason);
    } else if (state === "running") {
      state = "over";
      log("stopped", { backend, stage: reason.stage, message: reason.message });
      resolveEnded(reason);
    }
  }

  // A wasm trap after a panic arrives here; the panic hook has usually reported it already.
  function onUncaught(event) {
    const error = event.error ?? event.reason;
    if (error instanceof WebAssembly.RuntimeError) fail(failure("crash", backend, error, state === "running"));
  }

  let tornDown = false;
  function teardown() {
    if (tornDown) return;
    tornDown = true;
    state = "over";
    clearTimeout(watchdog);
    removeEventListener("error", onUncaught);
    removeEventListener("unhandledrejection", onUncaught);
    try {
      viewer?.stop();
    } catch (error) {
      log("stop-failed", { backend, message: error?.message ?? String(error) });
    }
    canvas.style.visibility = "hidden";
    setTimeout(() => {
      if (backend === "webgl") canvas.getContext("webgl2")?.getExtension("WEBGL_lose_context")?.loseContext();
      canvas.remove();
    }, RELEASE_MS);
  }

  addEventListener("error", onUncaught);
  addEventListener("unhandledrejection", onUncaught);
  mount(canvas);

  const step = async (stage, run) => {
    try {
      return await run();
    } catch (error) {
      throw failure(stage, backend, error);
    }
  };

  try {
    log("loading", { backend });
    const { factory, module } = await step("fetch", () => load(backend));
    const bindings = factory();
    await step("instantiate", () => bindings({ module_or_path: module }));
    log("instantiated", { backend });
    viewer = await step("start", () =>
      bindings.start(canvas, {
        ...options,
        strict: true,
        onReady() {
          if (state !== "starting") return;
          state = "running";
          clearTimeout(watchdog);
          log("ready", { backend });
          resolveReady(session);
        },
        onError: (reason) => fail(failure(reason.stage, backend, reason, reason.drawn)),
        onGpu(gpu) {
          session.gpu = gpu;
          log("gpu", { backend, ...gpu });
        },
      }),
    );
    session.viewer = viewer;
    if (state === "starting") {
      watchdog = setTimeout(
        () => fail(failure("first-frame", backend, `no frame was drawn within ${watchdogMs / 1000} s`)),
        watchdogMs,
      );
    }
  } catch (reason) {
    fail(reason);
  }
  return ready;
}
