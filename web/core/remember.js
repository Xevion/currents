/**
 * Which backend last worked in this browser, when it was not the first choice, so the next
 * visit skips the one that failed. Keyed to the user agent, since an update may fix it.
 */

const KEY = "currents.backend";

/** The backend to try first in the browser `agent`, or null. */
export function remembered(storage, agent) {
  try {
    const entry = JSON.parse(storage.getItem(KEY) ?? "null");
    return entry?.agent === agent && typeof entry.backend === "string" ? entry.backend : null;
  } catch {
    return null;
  }
}

/** Records that `backend` started in `agent`; forgets it when it is the first of `preferred`. */
export function remember(storage, agent, backend, preferred) {
  try {
    if (backend === preferred[0]) storage.removeItem(KEY);
    else storage.setItem(KEY, JSON.stringify({ agent, backend }));
  } catch {
    // Storage can be off or full; the next visit just probes again.
  }
}
