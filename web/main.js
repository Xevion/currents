const manifest = JSON.parse(document.getElementById("manifest").textContent);
const canvas = document.getElementById("stage");
const errorLine = document.getElementById("error");

function fail(error) {
  errorLine.textContent = error instanceof Error ? error.message : String(error);
  errorLine.hidden = false;
}

function webgl2Usable() {
  const gl = document.createElement("canvas").getContext("webgl2");
  const usable = gl !== null && gl.getExtension("EXT_color_buffer_float") !== null;
  gl?.getExtension("WEBGL_lose_context")?.loseContext();
  return usable;
}

async function webgpuUsable() {
  try {
    return Boolean(navigator.gpu && (await navigator.gpu.requestAdapter()));
  } catch {
    return false;
  }
}

// `?backend=webgpu` or `?backend=webgl` skips the probe.
async function pickBundle() {
  const forced = new URLSearchParams(location.search).get("backend");
  if (forced in manifest) return manifest[forced];
  if (await webgpuUsable()) return manifest.webgpu;
  return webgl2Usable() ? manifest.webgl : null;
}

// The viewer sizes the canvas from `devicePixelContentBoxSize` but scales input and the menu by
// `devicePixelRatio`. Where the two disagree (DevTools device emulation reports host pixels),
// hiding the former makes it size from CSS pixels times the ratio, so every measure agrees.
function devicePixelBoxAgrees() {
  if (!("devicePixelContentBoxSize" in ResizeObserverEntry.prototype)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const observer = new ResizeObserver(([entry]) => {
      observer.disconnect();
      const device = entry.devicePixelContentBoxSize[0].inlineSize;
      const css = entry.contentRect.width * devicePixelRatio;
      resolve(Math.abs(device - css) <= 2);
    });
    observer.observe(canvas, { box: "device-pixel-content-box" });
  });
}

async function run() {
  if (!(await devicePixelBoxAgrees())) {
    delete ResizeObserverEntry.prototype.devicePixelContentBoxSize;
  }
  const bundle = await pickBundle();
  if (bundle === null) {
    fail("This browser has neither WebGPU nor WebGL2 with float render targets.");
    return;
  }
  const glue = await import(`./${bundle.js}`);
  await glue.default({ module_or_path: new URL(bundle.wasm, import.meta.url) });
  glue.start(canvas, () => canvas.focus(), fail);
}

// The viewer leaves browser defaults alone; these block only the ones that fight it. Keys with
// Ctrl, Alt or Meta and the function keys stay with the browser, so reload and devtools work.
function viewerKey(event) {
  return !event.ctrlKey && !event.metaKey && !event.altKey && !/^F\d+$/.test(event.key);
}
canvas.addEventListener("keydown", (event) => viewerKey(event) && event.preventDefault());
canvas.addEventListener("wheel", (event) => event.preventDefault(), { passive: false });
canvas.addEventListener("pointerdown", () => canvas.focus());
document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener("gesturestart", (event) => event.preventDefault());
run().catch(fail);
