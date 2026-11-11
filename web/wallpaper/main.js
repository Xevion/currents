import { attempt } from "../core/attempt.js";
import { diagnostics } from "../core/diagnostics.js";
import { failure } from "../core/failure.js";
import { supported as probe } from "../core/probe.js";
import { remember, remembered } from "../core/remember.js";
import { NAMES, launch, order } from "../core/runner.js";
import { describeStage } from "../core/stages.js";
import { hostBridge } from "./bridge.js";
import { packageLoader } from "./loader.js";

/** A wallpaper may start while the machine is still busy logging in, so it gets longer. */
const WATCHDOG_MS = 45_000;
const PREFERRED = ["webgpu", "webgl"];
/**
 * A lost device is common on a desktop that sleeps and wakes or updates its driver, so the
 * same backend is started again a few times before the next one is tried.
 */
const POLICY = { restarts: 3, backoffMs: [2_000, 10_000, 30_000], stableMs: 10 * 60_000 };

const record = diagnostics("wallpaper");
const { log } = record;
// Hosts call their globals from the moment the page loads, so these go in before anything waits.
const bridge = hostBridge(log);
const manifest = JSON.parse(document.getElementById("manifest").textContent);
const screen = document.getElementById("screen");
const storage = (() => {
  try {
    return localStorage;
  } catch {
    return null;
  }
})();

function show(phase, text = "") {
  screen.hidden = phase === "running";
  screen.dataset.phase = phase;
  document.getElementById("screen-text").textContent = text;
}

function failed(failures) {
  const lines = failures.map((f) => `${NAMES[f.backend] ?? f.backend}: ${f.message || describeStage(f.stage)}`);
  show("failed", ["Currents could not start.", ...lines].join("\n"));
  console.error(`currents: could not start\n${record.text()}`);
}

async function run() {
  const recalled = storage && remembered(storage, navigator.userAgent);
  const loader = packageLoader({ manifest, log });
  loader.prefetch(recalled ?? ("gpu" in navigator ? "webgpu" : "webgl"));
  const supported = await probe(log);
  const backends = order({ supported, remembered: recalled });
  if (backends.length === 0) {
    failed([failure("probe", "webgl", "neither WebGPU nor WebGL2 with float render targets is available")]);
    return;
  }
  await launch({
    backends,
    policy: POLICY,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    now: () => performance.now(),
    start: (backend, { notice }) =>
      attempt({
        backend,
        load: loader.load,
        mount: (canvas) => document.body.prepend(canvas),
        options: { notice },
        watchdogMs: WATCHDOG_MS,
        log,
      }),
    onState(state) {
      if (state.phase === "running") {
        show("running");
        window.currents = state.session.viewer;
        bridge.attach(state.session.viewer);
        if (storage) remember(storage, navigator.userAgent, state.backend, PREFERRED);
      } else if (state.phase === "restarting" || (state.phase === "starting" && state.failures.length > 0)) {
        bridge.attach(null);
        window.currents = undefined;
        show("waiting");
      } else if (state.phase === "failed") {
        bridge.attach(null);
        window.currents = undefined;
        failed(state.failures);
      }
    },
  });
}

run().catch((error) => failed([failure("start", "page", error)]));
