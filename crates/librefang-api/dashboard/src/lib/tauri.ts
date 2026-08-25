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

let webCredentials: StoredCredentials | null = null;

function removeLegacyWebCredentials(): void {
  sessionStorage.removeItem("lf_creds");
}

export async function storeCredentials(creds: StoredCredentials): Promise<void> {
  if (!isMobileTauri()) {
    removeLegacyWebCredentials();
    webCredentials = { ...creds };
    return;
  }
  await invoke("store_credentials", {
    baseUrl: creds.base_url,
    apiKey: creds.api_key,
  });
}

export async function getCredentials(): Promise<StoredCredentials | null> {
  if (!isMobileTauri()) {
    removeLegacyWebCredentials();
    return webCredentials ? { ...webCredentials } : null;
  }
  return invoke<StoredCredentials | null>("get_credentials");
}

export async function clearCredentials(): Promise<void> {
  if (!isMobileTauri()) {
    removeLegacyWebCredentials();
    webCredentials = null;
    return;
  }
  await invoke("clear_credentials");
}

// ── Barcode scanner (mobile only) ─────────────────────────────────────────

export type QrScanResult =
  | { status: "success"; content: string }
  | { status: "unsupported" }
  | { status: "cancelled" }
  | { status: "error"; error: unknown };

export async function scanQrCode(): Promise<QrScanResult> {
  if (!isMobileTauri()) return { status: "unsupported" };
  try {
    const result = await invoke<{ content: string }>(
      "plugin:barcode-scanner|scan",
      { formats: ["QR_CODE"] },
    );
    return result?.content
      ? { status: "success", content: result.content }
      : { status: "cancelled" };
  } catch (error) {
    return { status: "error", error };
  }
}
