// Platform detection + Tauri invoke helpers.
// Falls back gracefully in browser (non-Tauri) builds.

interface TauriCoreApi {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
}

declare global {
  interface Window {
    __TAURI__?: { core: TauriCoreApi };
  }
}

export const isTauri = (): boolean =>
  typeof window !== "undefined" && !!window.__TAURI__;

export const isMobileTauri = (): boolean =>
  isTauri() &&
  (/Android/.test(navigator.userAgent) ||
    /iPhone|iPad|iPod/.test(navigator.userAgent));

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!window.__TAURI__) throw new Error("Not running in Tauri");
  return window.__TAURI__.core.invoke(cmd, args) as Promise<T>;
}

// ── Credential storage (wraps Tauri keyring commands) ─────────────────────

export interface StoredCredentials {
  base_url: string;
  api_key: string;
}

export async function storeCredentials(creds: StoredCredentials): Promise<void> {
  if (!isMobileTauri()) {
    sessionStorage.setItem("lf_creds", JSON.stringify(creds));
    return;
  }
  await invoke("store_credentials", {
    baseUrl: creds.base_url,
    apiKey: creds.api_key,
  });
}

export async function getCredentials(): Promise<StoredCredentials | null> {
  if (!isMobileTauri()) {
    const raw = sessionStorage.getItem("lf_creds");
    return raw ? (JSON.parse(raw) as StoredCredentials) : null;
  }
  try {
    return await invoke<StoredCredentials | null>("get_credentials");
  } catch {
    return null;
  }
}

export async function clearCredentials(): Promise<void> {
  if (!isMobileTauri()) {
    sessionStorage.removeItem("lf_creds");
    return;
  }
  try {
    await invoke("clear_credentials");
  } catch {
    // ignore if nothing stored
  }
}

// ── Barcode scanner (mobile only) ─────────────────────────────────────────

export async function scanQrCode(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const result = await invoke<{ content: string }>(
      "plugin:barcode-scanner|scan",
      { formats: ["QR_CODE"] },
    );
    return result?.content ?? null;
  } catch {
    return null;
  }
}
