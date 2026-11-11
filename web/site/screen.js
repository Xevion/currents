import { NAMES } from "../core/runner.js";
import { describeStage } from "../core/stages.js";

/**
 * The loading and error screen over the canvas. It is in the page's HTML, so it shows from
 * the first paint; this only changes what it says and takes it away once a frame is drawn.
 */
export function siteScreen({ diagnostics, backends }) {
  const root = document.getElementById("screen");
  const status = document.getElementById("screen-status");
  const bar = document.getElementById("screen-progress");
  const failure = document.getElementById("screen-failure");
  const summary = document.getElementById("failure-summary");
  const list = document.getElementById("failure-list");
  const details = document.getElementById("failure-details");
  const actions = document.getElementById("failure-actions");

  const show = (phase) => (root.dataset.phase = phase);

  function button(label, onClick) {
    const element = document.createElement("button");
    element.type = "button";
    element.textContent = label;
    element.addEventListener("click", () => onClick(element));
    return element;
  }

  function withBackend(backend) {
    const url = new URL(location.href);
    if (backend) url.searchParams.set("backend", backend);
    else url.searchParams.delete("backend");
    return url.toString();
  }

  return {
    root,
    say(text) {
      root.hidden = false;
      show("loading");
      status.textContent = text;
    },
    progress(backend, loaded, total) {
      if (!total) return;
      const fraction = Math.min(loaded / total, 1);
      bar.hidden = false;
      bar.value = fraction;
      status.textContent = `Downloading the ${NAMES[backend]} renderer, ${Math.round(fraction * 100)}%`;
    },
    ready() {
      show("ready");
      // Hidden after the fade; the timer covers reduced motion, where no transition runs.
      setTimeout(() => root.dataset.phase === "ready" && (root.hidden = true), 600);
    },
    failed(failures, { pinned, supported }) {
      root.hidden = false;
      show("failed");
      if (failures.length === 0) {
        summary.textContent = "This browser has neither WebGPU nor WebGL2 with float render targets, which Currents needs to draw.";
      } else {
        summary.textContent = pinned
          ? `The ${NAMES[pinned]} renderer was asked for in the link, and it failed.`
          : "Every way this browser offers to draw failed.";
      }
      list.replaceChildren(
        ...failures.map((f) => {
          const item = document.createElement("li");
          const name = document.createElement("strong");
          name.textContent = NAMES[f.backend] ?? f.backend;
          // The viewer's own message says what failed; the stage stands in when there is none.
          item.append(name, `: ${f.message || describeStage(f.stage)}`);
          return item;
        }),
      );
      actions.replaceChildren(button("Retry", () => location.reload()));
      for (const backend of backends) {
        if (backend !== pinned && (pinned || supported.includes(backend))) {
          actions.append(button(`Try ${NAMES[backend]}`, () => location.assign(withBackend(backend))));
        }
      }
      if (pinned) actions.append(button("Choose automatically", () => location.assign(withBackend(null))));
      actions.append(
        button("Copy diagnostics", async (element) => {
          try {
            await navigator.clipboard.writeText(diagnostics.text());
            element.textContent = "Copied";
          } catch {
            element.textContent = "Copy failed; see Details";
          }
        }),
      );
      details.textContent = diagnostics.text();
      failure.hidden = false;
      failure.querySelector("button")?.focus();
    },
  };
}
