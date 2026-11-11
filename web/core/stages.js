/** Plain words for each failure stage, for screens a person reads. */
const STAGES = {
  probe: "is not available here",
  fetch: "could not be downloaded",
  compile: "could not be compiled, which usually means the browser is too old",
  instantiate: "could not be set up",
  start: "could not start",
  window: "could not use the page's canvas",
  surface: "could not draw to the page",
  device: "could not open the graphics device",
  shaders: "had shaders the graphics driver rejected",
  validation: "hit a graphics error",
  lost: "lost its graphics device",
  crash: "crashed",
  "first-frame": "never finished drawing a frame",
};

export function describeStage(stage) {
  return STAGES[stage] ?? `failed (${stage})`;
}
