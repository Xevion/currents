/**
 * Decides which backend runs and what happens when one fails, with every effect passed in, so a
 * frontend supplies how backends start and what failure costs it.
 */

/** Backends from most to least preferred. */
const PREFERENCE = ["webgpu", "webgl"];

export const NAMES = { webgpu: "WebGPU", webgl: "WebGL2" };

/** The backends to try, in order. */
export function order({ supported, pinned, remembered }) {
  if (pinned) return [pinned];
  const ranked = PREFERENCE.filter((backend) => supported.includes(backend));
  if (remembered && ranked.includes(remembered)) {
    return [remembered, ...ranked.filter((backend) => backend !== remembered)];
  }
  return ranked;
}

/** What to tell the person when `backend` starts after `failures`; null when nothing changed for them. */
export function notice(failures, backend) {
  const last = failures.at(-1);
  if (!last || last.backend === backend) return null;
  const what = last.drawn ? "stopped" : "did not start";
  const reason = last.message.replace(/\.$/, "");
  return `${NAMES[last.backend]} ${what} (${reason}); running on ${NAMES[backend]} instead.`;
}

/**
 * Runs `backends` in order until one starts, and moves on whenever a running one stops. Each
 * call `start(backend, {notice})` resolves to a session `{ended, stop()}` once it draws, or
 * rejects with a failure; `ended` resolves with the failure that stopped it.
 *
 * `policy.restarts` is how many times a stopped backend is started again, after
 * `policy.backoffMs[n]`, before the next is tried; a session that ran `policy.stableMs` clears
 * the count. A `pinned` run never moves to another backend. Resolves once nothing is left to try.
 */
export async function launch({ backends, pinned = false, start, policy, sleep, now, onState }) {
  const failures = [];
  const backoff = policy.backoffMs ?? [];
  let index = 0;
  let restarts = 0;
  while (index < backends.length) {
    const backend = backends[index];
    onState({ phase: "starting", backend, failures: [...failures] });
    let session;
    try {
      session = await start(backend, { notice: notice(failures, backend) });
    } catch (failure) {
      failures.push(failure);
      index++;
      restarts = 0;
      continue;
    }
    const began = now();
    onState({ phase: "running", backend, failures: [...failures], session });
    const failure = await session.ended;
    session.stop();
    failures.push(failure);
    if (now() - began >= policy.stableMs) restarts = 0;
    if (restarts < policy.restarts) {
      onState({ phase: "restarting", backend, failures: [...failures] });
      await sleep(backoff[Math.min(restarts, backoff.length - 1)] ?? 0);
      restarts++;
      continue;
    }
    if (pinned) break;
    index++;
    restarts = 0;
  }
  onState({ phase: "failed", failures: [...failures] });
}
