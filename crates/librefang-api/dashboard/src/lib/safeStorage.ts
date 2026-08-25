// Guarded localStorage access.
//
// `localStorage` is not always available or safe to touch:
//   - Safari private mode throws `SecurityError` on read AND write.
//   - `setItem` throws `QuotaExceededError` synchronously when full.
//   - Server-side / non-browser contexts have no `window.localStorage`.
//
// An unguarded access in module init or a `useState` initializer takes
// down the whole React tree on first paint. These helpers mirror the
// try/catch pattern already used in `components/TerminalTabs.tsx`
// (#5140) so every site degrades to a sensible default instead.

export type StorageReadResult =
  | { ok: true; value: string | null }
  | { ok: false; reason: "unavailable" | "error"; error?: unknown };

type StorageAccessResult<T> =
  | { ok: true; value: T }
  | { ok: false; reason: "unavailable" | "error"; error?: unknown };

function withStorage<T>(
  operation: "safeStorageGet" | "safeStorageSet",
  key: string,
  access: (storage: Storage) => T,
  warnUnavailable: boolean,
): StorageAccessResult<T> {
  let storage: Storage | undefined;
  try {
    storage = globalThis.localStorage;
  } catch (error) {
    console.warn(`${operation}("${key}") failed:`, error);
    return { ok: false, reason: "error", error };
  }
  if (typeof storage === "undefined") {
    if (warnUnavailable) {
      console.warn(`${operation}("${key}") skipped: localStorage unavailable`);
    }
    return { ok: false, reason: "unavailable" };
  }
  try {
    return { ok: true, value: access(storage) };
  } catch (error) {
    console.warn(`${operation}("${key}") failed:`, error);
    return { ok: false, reason: "error", error };
  }
}

export function safeStorageRead(key: string): StorageReadResult {
  return withStorage("safeStorageGet", key, (storage) => storage.getItem(key), false);
}

export function safeStorageGet(key: string): string | null {
  const result = safeStorageRead(key);
  return result.ok ? result.value : null;
}

export function safeStorageSet(key: string, value: string): void {
  withStorage("safeStorageSet", key, (storage) => storage.setItem(key, value), true);
}
