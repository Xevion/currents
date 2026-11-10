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

async function run() {
  const bundle = await pickBundle();
  if (bundle === null) {
    fail("This browser has neither WebGPU nor WebGL2 with float render targets.");
    return;
  }
  const glue = await import(`./${bundle.js}`);
  await glue.default({ module_or_path: new URL(bundle.wasm, import.meta.url) });
  glue.start(canvas, () => canvas.focus(), fail);
}

document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener("gesturestart", (event) => event.preventDefault());
run().catch(fail);
