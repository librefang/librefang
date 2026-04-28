import { useState, useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Wifi, QrCode, Loader2, CheckCircle, AlertCircle, RefreshCw } from "lucide-react";
import { isMobileTauri, scanQrCode, storeCredentials, getCredentials } from "../lib/tauri";

type Tab = "manual" | "qr";
type Step = "idle" | "scanning" | "connecting" | "done" | "error";

function navigateToDashboard(baseUrl: string) {
  window.location.href = baseUrl.replace(/\/$/, "") + "/dashboard";
}

export function ConnectWizardPage() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>("manual");
  const [step, setStep] = useState<Step>("idle");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [errorMsg, setErrorMsg] = useState("");

  // Already connected → skip wizard
  useEffect(() => {
    getCredentials().then((creds) => {
      if (creds) {
        if (isMobileTauri()) {
          navigateToDashboard(creds.base_url);
        } else {
          void navigate({ to: "/overview" });
        }
      }
    });
  }, [navigate]);

  async function connectManual() {
    const url = baseUrl.trim().replace(/\/$/, "");
    const key = apiKey.trim();
    if (!url || !key) return;
    setStep("connecting");
    setErrorMsg("");
    try {
      const resp = await fetch(`${url}/api/health`, {
        headers: { Authorization: `Bearer ${key}` },
        signal: AbortSignal.timeout(10_000),
      });
      if (!resp.ok) throw new Error(`Server returned ${resp.status}`);
      await storeCredentials({ base_url: url, api_key: key });
      setStep("done");
      setTimeout(() => navigateToDashboard(url), 1200);
    } catch (err: unknown) {
      setStep("error");
      setErrorMsg(err instanceof Error ? err.message : "Connection failed");
    }
  }

  async function connectQr() {
    setStep("scanning");
    setErrorMsg("");
    try {
      const raw = await scanQrCode();
      if (!raw) {
        setStep("idle");
        return;
      }

      setStep("connecting");

      // Parse librefang://pair?payload=<base64url-no-pad>
      const uri = new URL(raw);
      const payloadB64 = uri.searchParams.get("payload");
      if (!payloadB64) throw new Error("Invalid QR code: missing payload");

      // base64url (no-pad) → standard base64 with padding → JSON
      const stdB64 = payloadB64.replace(/-/g, "+").replace(/_/g, "/");
      const padded = stdB64.padEnd(Math.ceil(stdB64.length / 4) * 4, "=");
      const payloadJson = atob(padded);
      const payload = JSON.parse(payloadJson) as {
        v: number;
        base_url: string;
        token: string;
        expires_at: string;
      };

      if (payload.v !== 1) throw new Error("Unsupported QR format version");
      if (new Date(payload.expires_at).getTime() < Date.now()) {
        throw new Error("QR code has expired — refresh it on the desktop");
      }

      const pairingUrl = payload.base_url.replace(/\/$/, "");
      if (!pairingUrl.startsWith("http://") && !pairingUrl.startsWith("https://")) {
        throw new Error("Invalid QR code: unexpected base_url protocol");
      }
      const platform = /Android/.test(navigator.userAgent) ? "android" : "ios";

      const res = await fetch(`${pairingUrl}/api/pairing/complete`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          token: payload.token,
          display_name: "LibreFang Mobile",
          platform,
        }),
        signal: AbortSignal.timeout(15_000),
      });

      if (res.status === 410) throw new Error("Pairing token expired or already used");
      if (!res.ok) {
        const body = await res.json().catch(() => ({})) as { error?: string };
        throw new Error(body.error ?? `Server returned ${res.status}`);
      }

      const result = await res.json() as { api_key: string };
      await storeCredentials({ base_url: pairingUrl, api_key: result.api_key });
      setStep("done");
      setTimeout(() => navigateToDashboard(pairingUrl), 1200);
    } catch (err: unknown) {
      setStep("error");
      setErrorMsg(err instanceof Error ? err.message : "Pairing failed");
    }
  }

  function reset() {
    setStep("idle");
    setErrorMsg("");
  }

  if (step === "done") {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center bg-main gap-4 px-6">
        <CheckCircle className="w-14 h-14 text-success" />
        <p className="text-xl font-bold">Connected!</p>
        <p className="text-sm text-text-dim">Opening dashboard…</p>
      </div>
    );
  }

  const busy = step === "scanning" || step === "connecting";

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-main px-6 py-12">
      <div className="w-full max-w-sm space-y-8">
        {/* Header */}
        <div className="text-center space-y-2">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-brand/10 ring-2 ring-brand/20">
            <Wifi className="h-7 w-7 text-brand" />
          </div>
          <h1 className="text-2xl font-black tracking-tight">Connect to Daemon</h1>
          <p className="text-sm text-text-dim">
            Enter your daemon URL and API key, or scan a pairing QR code.
          </p>
        </div>

        {/* Tab switcher */}
        <div className="grid grid-cols-2 gap-1 rounded-xl bg-surface p-1 border border-border-subtle">
          <button
            onClick={() => { setTab("manual"); reset(); }}
            disabled={busy}
            className={`rounded-lg py-2 text-sm font-semibold transition-colors ${
              tab === "manual"
                ? "bg-brand text-white shadow-sm"
                : "text-text-dim hover:text-brand disabled:opacity-50"
            }`}
          >
            Manual
          </button>
          <button
            onClick={() => { setTab("qr"); reset(); }}
            disabled={busy}
            className={`rounded-lg py-2 text-sm font-semibold transition-colors ${
              tab === "qr"
                ? "bg-brand text-white shadow-sm"
                : "text-text-dim hover:text-brand disabled:opacity-50"
            }`}
          >
            Scan QR
          </button>
        </div>

        {/* Tab content */}
        {tab === "manual" ? (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor="daemon-url" className="text-xs font-semibold text-text-dim uppercase tracking-wider">
                Daemon URL
              </label>
              <input
                id="daemon-url"
                type="url"
                inputMode="url"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                placeholder="http://192.168.1.100:4545"
                value={baseUrl}
                onChange={(e) => { setBaseUrl(e.target.value); reset(); }}
                disabled={busy}
                className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors placeholder:text-text-dim/40 disabled:opacity-50"
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="api-key" className="text-xs font-semibold text-text-dim uppercase tracking-wider">
                API Key
              </label>
              <input
                id="api-key"
                type="password"
                placeholder="••••••••••••••••"
                value={apiKey}
                onChange={(e) => { setApiKey(e.target.value); reset(); }}
                disabled={busy}
                className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors placeholder:text-text-dim/40 disabled:opacity-50"
              />
            </div>
            <button
              onClick={() => void connectManual()}
              disabled={busy || !baseUrl.trim() || !apiKey.trim()}
              className="w-full rounded-xl bg-brand py-3 text-sm font-bold text-white hover:bg-brand/90 transition-colors shadow-lg shadow-brand/20 disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
            >
              {step === "connecting" ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Connecting…
                </>
              ) : (
                <>
                  Connect
                  <Wifi className="w-4 h-4" />
                </>
              )}
            </button>
          </div>
        ) : (
          <div className="space-y-4">
            <div className="rounded-2xl border border-border-subtle bg-surface p-6 text-center space-y-3">
              <QrCode className="w-10 h-10 mx-auto text-text-dim" />
              <div className="text-sm text-text-dim space-y-1">
                <p className="font-medium">Open the desktop dashboard</p>
                <p>
                  Go to <span className="font-semibold text-brand">Settings → Mobile Pairing</span>
                  {" "}and tap <strong>Scan QR</strong> below.
                </p>
              </div>
            </div>
            <button
              onClick={() => void connectQr()}
              disabled={busy}
              className="w-full rounded-xl bg-brand py-3 text-sm font-bold text-white hover:bg-brand/90 transition-colors shadow-lg shadow-brand/20 disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
            >
              {step === "scanning" ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Scanning…
                </>
              ) : step === "connecting" ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Pairing…
                </>
              ) : (
                <>
                  <QrCode className="w-4 h-4" />
                  Scan QR Code
                </>
              )}
            </button>
          </div>
        )}

        {/* Error state */}
        {step === "error" && (
          <div className="rounded-xl border border-error/20 bg-error/5 p-4 space-y-2">
            <div className="flex items-center gap-2 text-error">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <p className="text-sm font-semibold">Connection failed</p>
            </div>
            <p className="text-xs text-text-dim">{errorMsg}</p>
            <button
              onClick={reset}
              className="flex items-center gap-1.5 text-xs text-brand hover:underline"
            >
              <RefreshCw className="w-3 h-3" />
              Try again
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
