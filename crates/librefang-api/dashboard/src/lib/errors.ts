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

function messageWithCause(err: Error, includeCause: boolean): string {
  if (!includeCause) return err.message;
  const extra = deepestCauseMessage(err);
  return extra ? `${err.message}: ${extra}` : err.message;
}

/** @internal Exported for explicit production/development contract tests. */
export function formatToastError(
  err: unknown,
  fallback: string,
  includeCause: boolean,
): string {
  if (err instanceof ApiError) {
    return `[${err.status}] ${messageWithCause(err, includeCause)}`;
  }

  if (err instanceof Error && err.message) {
    return messageWithCause(err, includeCause);
  }

  if (typeof err === "string" && err) return err;
  return fallback;
}

/**
 * Extract a user-facing error message from an unknown thrown value.
 *
 * Priority order (highest → lowest):
 *  1. ApiError — includes status code and its public message
 *  2. Error instance — includes its public message
 *  3. Raw string — returned as-is
 *  4. Fallback — caller-provided default
 */
export function toastErr(err: unknown, fallback: string): string {
  if (import.meta.env.DEV) {
    console.error("[toastErr]", err);
  }

  return formatToastError(err, fallback, import.meta.env.DEV);
}
