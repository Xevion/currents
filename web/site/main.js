import { attempt } from "../core/attempt.js";
import { diagnostics } from "../core/diagnostics.js";
import { failure } from "../core/failure.js";
import { supported as probe } from "../core/probe.js";
import { remember, remembered } from "../core/remember.js";
import { NAMES, launch, order } from "../core/runner.js";
import { agreeOnPixelSize, shimUncapturedError } from "./browser.js";
import { networkLoader } from "./loader.js";
import { siteScreen } from "./screen.js";

/** How long a backend may take from starting to its first finished frame. */
const WATCHDOG_MS = 20_000;
const PREFERRED = ["webgpu", "webgl"];

const manifest = JSON.parse(document.getElementById("manifest").textContent);
const record = diagnostics("site");
const { log } = record;
const screen = siteScreen({ diagnostics: record, backends: PREFERRED });
const storage = (() => {
  try {
    return localStorage;
  } catch {
    return null;
  }
})();

// The viewer leaves browser defaults alone; these block only the ones that fight it. Keys with
// Ctrl, Alt or Meta and the function keys stay with the browser, so reload and devtools work.
function viewerKey(event) {
  return !event.ctrlKey && !event.metaKey && !event.altKey && !/^F\d+$/.test(event.key);
}

function mount(canvas) {
  canvas.addEventListener("keydown", (event) => viewerKey(event) && event.preventDefault());
  canvas.addEventListener("wheel", (event) => event.preventDefault(), { passive: false });
  canvas.addEventListener("pointerdown", () => canvas.focus());
  document.body.prepend(canvas);
}

async function run() {
  log("page", { shim: shimUncapturedError() });
  const asked = new URLSearchParams(location.search).get("backend");
  const pinned = asked in manifest ? asked : null;
  const recalled = storage && remembered(storage, navigator.userAgent);
  const loader = networkLoader({ manifest, log, onProgress: screen.progress });

  // Download the likeliest bundle while the probe runs, since asking for an adapter can be slow.
  loader.prefetch(pinned ?? recalled ?? ("gpu" in navigator ? "webgpu" : "webgl"));
  screen.say("Checking what this browser can draw with");
  const [agree, supported] = await Promise.all([agreeOnPixelSize(screen.root), probe(log)]);
  if (!agree) log("pixel-size", { note: "device pixels disagree with CSS pixels; sizing from CSS" });

  const backends = order({ supported, pinned, remembered: recalled });
  if (backends.length === 0) {
    screen.failed([], { pinned, supported });
    return;
  }
  await launch({
    backends,
    pinned: Boolean(pinned),
    policy: { restarts: 0 },
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    now: () => performance.now(),
    start(backend, { notice }) {
      screen.say(`Starting the ${NAMES[backend]} renderer`);
      return attempt({ backend, load: loader.load, mount, options: { notice, share: true }, watchdogMs: WATCHDOG_MS, log });
    },
    onState(state) {
      if (state.phase === "running") {
        window.currents = state.session.viewer;
        state.session.canvas.focus();
        screen.ready();
        if (!pinned && storage) remember(storage, navigator.userAgent, state.backend, PREFERRED);
      } else if (state.phase === "failed") {
        window.currents = undefined;
        screen.failed(state.failures, { pinned, supported });
      }
    },
  });
}

document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener("gesturestart", (event) => event.preventDefault());
run().catch((error) => {
  log("page-failed", { message: error?.message ?? String(error) });
  screen.failed([failure("start", "page", error)], { pinned: null, supported: [] });
});
