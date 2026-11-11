/**
 * A failure is `{stage, backend, message, drawn, detail?}`: where it happened, on which backend,
 * what went wrong, and whether a frame had been drawn first. The viewer reports its own in
 * that shape; everything else that can throw is wrapped into one here.
 */

/** `error` from `stage` as a failure, unless it already is one. */
export function failure(stage, backend, error, drawn = false) {
  if (error && typeof error === "object" && typeof error.stage === "string" && typeof error.message === "string") {
    return { drawn, ...error, backend };
  }
  const message = error instanceof Error ? error.message : String(error);
  const detail = error instanceof Error && error.stack ? error.stack : undefined;
  return { stage, backend, message, drawn, detail };
}
