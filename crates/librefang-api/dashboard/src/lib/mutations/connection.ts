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

const HEALTH_TIMEOUT_MS = 10_000;
const PAIR_TIMEOUT_MS = 15_000;

async function healthCheck({ baseUrl, apiKey }: ManualConnectInput): Promise<void> {
  const resp = await fetch(`${baseUrl}/api/health`, {
    headers: { Authorization: `Bearer ${apiKey}` },
    signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
  });
  if (!resp.ok) throw new Error(`Server returned ${resp.status}`);
}

async function exchangePairingToken(input: QrConnectInput): Promise<QrConnectResult> {
  const res = await fetch(`${input.baseUrl}/api/pairing/complete`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      token: input.token,
      display_name: input.displayName,
      platform: input.platform,
    }),
    signal: AbortSignal.timeout(PAIR_TIMEOUT_MS),
  });
  if (res.status === 410) throw new Error("Pairing token expired or already used");
  if (!res.ok) {
    const body = (await res.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error ?? `Server returned ${res.status}`);
  }
  const result = (await res.json()) as { api_key: string };
  return { baseUrl: input.baseUrl, apiKey: result.api_key };
}

/**
 * Manual connect: validate credentials against the daemon, then persist them.
 */
export function useConnectManual() {
  return useMutation({
    mutationFn: async (input: ManualConnectInput) => {
      await healthCheck(input);
      await storeCredentials({ base_url: input.baseUrl, api_key: input.apiKey });
      return input;
    },
  });
}

/**
 * QR connect: redeem the one-time pairing token at the daemon, store the
 * returned per-pairing api_key.
 */
export function useConnectViaQr() {
  return useMutation({
    mutationFn: async (input: QrConnectInput): Promise<QrConnectResult> => {
      const result = await exchangePairingToken(input);
      await storeCredentials({ base_url: result.baseUrl, api_key: result.apiKey });
      return result;
    },
  });
}
