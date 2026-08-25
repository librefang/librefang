import { useMutation } from "@tanstack/react-query";
import { storeCredentials } from "../tauri";

// Mobile-side pairing flow. The daemon URL is provided per-call (the user
// supplies it manually or scans a QR code), so these mutations issue
// cross-origin fetches against an arbitrary base URL — they intentionally
// do NOT route through `src/api.ts`, which is wired to the SPA's own origin.

interface ManualConnectInput {
  baseUrl: string;
  apiKey: string;
}

interface QrConnectInput {
  baseUrl: string;
  token: string;
  displayName: string;
  platform: string;
}

interface QrConnectResult {
  baseUrl: string;
  apiKey: string;
}

interface ServerTarget {
  baseUrl: string;
  endpoint(path: string): string;
}

const HEALTH_TIMEOUT_MS = 10_000;
const PAIR_TIMEOUT_MS = 15_000;

function parseServerTarget(rawBaseUrl: string): ServerTarget {
  let url: URL;
  try {
    url = new URL(rawBaseUrl);
  } catch {
    throw new Error("Invalid server URL");
  }
  if (!(["http:", "https:"] as const).includes(url.protocol as "http:" | "https:")) {
    throw new Error("Server URL must use HTTP or HTTPS");
  }
  if (url.username || url.password) {
    throw new Error("Server URL must not include credentials");
  }
  if (url.search || url.hash) {
    throw new Error("Server URL must not include a query or fragment");
  }
  url.pathname = `${url.pathname.replace(/\/+$/, "")}/`;
  const baseUrl = url.toString().replace(/\/$/, "");
  return {
    baseUrl,
    endpoint: (path) => new URL(path.replace(/^\//, ""), url).toString(),
  };
}

async function fetchConnection(url: string, init: RequestInit, timeoutMessage: string) {
  try {
    return await fetch(url, init);
  } catch (error) {
    if (error instanceof DOMException && ["AbortError", "TimeoutError"].includes(error.name)) {
      throw new Error(timeoutMessage);
    }
    throw new Error("Could not reach the server. Please verify the address.");
  }
}

async function healthCheck({ baseUrl, apiKey }: ManualConnectInput): Promise<void> {
  const target = parseServerTarget(baseUrl);
  const resp = await fetchConnection(target.endpoint("api/health"), {
    headers: { Authorization: `Bearer ${apiKey}` },
    signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
  }, "Connection timed out. Please check the server address and try again.");
  if (!resp.ok) throw new Error(`Server returned ${resp.status}`);
}

async function exchangePairingToken(input: QrConnectInput): Promise<QrConnectResult> {
  const target = parseServerTarget(input.baseUrl);
  const res = await fetchConnection(target.endpoint("api/pairing/complete"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      token: input.token,
      display_name: input.displayName,
      platform: input.platform,
    }),
    signal: AbortSignal.timeout(PAIR_TIMEOUT_MS),
  }, "Pairing timed out. Refresh the QR code and try again.");
  if (res.status === 410) throw new Error("Pairing token expired or already used");
  if (!res.ok) {
    const body = (await res.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error ?? `Server returned ${res.status}`);
  }
  const result = (await res.json()) as { api_key?: unknown };
  if (typeof result.api_key !== "string" || result.api_key.trim() === "") {
    throw new Error("Server returned an invalid pairing response");
  }
  return { baseUrl: target.baseUrl, apiKey: result.api_key };
}

/**
 * Manual connect: validate credentials against the daemon, then persist them.
 */
export function useConnectManual() {
  return useMutation({
    mutationFn: async (input: ManualConnectInput) => {
      await healthCheck(input);
      const target = parseServerTarget(input.baseUrl);
      await storeCredentials({ base_url: target.baseUrl, api_key: input.apiKey });
      return { baseUrl: target.baseUrl };
    },
  });
}

/**
 * QR connect: redeem the one-time pairing token at the daemon, store the
 * returned per-pairing api_key.
 */
export function useConnectViaQr() {
  return useMutation({
    mutationFn: async (input: QrConnectInput): Promise<{ baseUrl: string }> => {
      const result = await exchangePairingToken(input);
      await storeCredentials({ base_url: result.baseUrl, api_key: result.apiKey });
      return { baseUrl: result.baseUrl };
    },
  });
}
