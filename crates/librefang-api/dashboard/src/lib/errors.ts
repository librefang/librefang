/**
 * Extract a human-readable message from a `catch (err: unknown)` value
 * (the default since `useUnknownInCatchVariables` / TS 4.4) or a
 * react-query mutation `onError(err)` callback (where `err` is `Error`
 * for our wire client but could be a thrown string from misbehaving
 * deps).
 *
 * The fallback is the caller's localized string; pass it from
 * `t("…")` so the error UI stays translated when the throw isn't an
 * `Error`. Empty `Error.message` falls through to the fallback rather
 * than rendering an empty toast.
 */
export function toastErr(err: unknown, fallback: string): string {
  if (err instanceof Error && err.message) return err.message;
  if (typeof err === "string" && err) return err;
  return fallback;
}
