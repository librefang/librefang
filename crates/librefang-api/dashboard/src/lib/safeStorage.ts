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

export function safeStorageGet(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch (e) {
    console.warn(`safeStorageGet("${key}") failed:`, e);
    return null;
  }
}

export function safeStorageSet(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch (e) {
    console.warn(`safeStorageSet("${key}") failed:`, e);
  }
}
