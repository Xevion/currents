/** What the browser can draw with, checked before anything is downloaded or started. */

/** Whether WebGPU gives an adapter, and why not when it does not. */
export async function webgpu() {
  if (!navigator.gpu) return { ok: false, reason: "navigator.gpu is missing" };
  try {
    const adapter = await navigator.gpu.requestAdapter();
    return adapter ? { ok: true } : { ok: false, reason: "no adapter" };
  } catch (error) {
    return { ok: false, reason: error?.message ?? String(error) };
  }
}

/** Whether WebGL2 can render to float textures, which the flow field is stored in. */
export function webgl() {
  const gl = document.createElement("canvas").getContext("webgl2");
  if (!gl) return { ok: false, reason: "no WebGL2 context" };
  const ok = gl.getExtension("EXT_color_buffer_float") !== null;
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return ok ? { ok } : { ok: false, reason: "no EXT_color_buffer_float" };
}

/** The backends the browser supports, logging what each check found. */
export async function supported(log) {
  const [gpu, gl] = [await webgpu(), webgl()];
  log("probe", { webgpu: gpu.ok || gpu.reason, webgl: gl.ok || gl.reason });
  return [gpu.ok && "webgpu", gl.ok && "webgl"].filter(Boolean);
}
