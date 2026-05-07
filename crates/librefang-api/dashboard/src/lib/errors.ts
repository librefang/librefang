import { ApiError } from "./http/errors";

const MAX_CAUSE_DEPTH = 5;

type ErrorWithCause = Error & { cause?: unknown };

function deepestCauseMessage(err: Error): string | undefined {
  let cur = (err as ErrorWithCause).cause;
  let found: string | undefined;
  let depth = 0;
  while (cur instanceof Error && depth < MAX_CAUSE_DEPTH) {
    if (cur.message && cur.message !== err.message) {
      found = cur.message;
    }
    cur = (cur as ErrorWithCause).cause;
    depth++;
  }
  return found;
}

/**
 * Extract a user-facing error message from an unknown thrown value.
 *
 * Priority order (highest → lowest):
 *  1. ApiError — includes status code + deepest cause message
 *  2. Error instance — message + deepest cause message
 *  3. Raw string — returned as-is
 *  4. Fallback — caller-provided default
 */
export function toastErr(err: unknown, fallback: string): string {
  if (import.meta.env.DEV) {
    console.error("[toastErr]", err);
  }

  if (err instanceof ApiError) {
    const extra = deepestCauseMessage(err);
    const body = extra ? `${err.message}: ${extra}` : err.message;
    return `[${err.status}] ${body}`;
  }

  if (err instanceof Error && err.message) {
    const extra = deepestCauseMessage(err);
    return extra ? `${err.message}: ${extra}` : err.message;
  }

  if (typeof err === "string" && err) return err;
  return fallback;
}
