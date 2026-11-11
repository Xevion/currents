/**
 * An ordered record of how the page started: each step with its time since navigation, what
 * the browser offered, the GPU the viewer opened, and every failure. Kept on
 * `window.currentsDiagnostics` and copied whole by the error screen.
 */

function environment(frontend) {
  const nav = globalThis.navigator ?? {};
  return {
    frontend,
    userAgent: nav.userAgent,
    platform: nav.userAgentData?.platform ?? nav.platform,
    pixelRatio: globalThis.devicePixelRatio,
    screen: globalThis.screen ? `${screen.width}x${screen.height}` : undefined,
    viewport: `${globalThis.innerWidth}x${globalThis.innerHeight}`,
    cores: nav.hardwareConcurrency,
    memoryGb: nav.deviceMemory,
    webgpu: "gpu" in nav,
    isolated: globalThis.crossOriginIsolated,
  };
}

export function diagnostics(frontend) {
  const entries = [];
  const env = environment(frontend);

  function log(event, data = {}) {
    const entry = { at: Math.round(performance.now()), event, ...data };
    entries.push(entry);
    console.debug(`currents: ${event}`, data);
    return entry;
  }

  function text() {
    const lines = Object.entries(env).map(([key, value]) => `${key}: ${value}`);
    for (const { at, event, ...data } of entries) {
      const rest = Object.keys(data).length ? ` ${JSON.stringify(data)}` : "";
      lines.push(`${String(at).padStart(6)} ms  ${event}${rest}`);
    }
    return lines.join("\n");
  }

  const record = { environment: env, entries, log, text };
  globalThis.currentsDiagnostics = record;
  return record;
}
