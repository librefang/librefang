import { useState, useEffect } from "react";
import { Wifi, QrCode, Loader2, CheckCircle, AlertCircle, RefreshCw } from "lucide-react";
import { isMobileTauri, scanQrCode, getCredentials, clearCredentials } from "../lib/tauri";
import { useConnectManual, useConnectViaQr } from "../lib/mutations/connection";

type Tab = "manual" | "qr";
type Step = "idle" | "scanning" | "connecting" | "done" | "error";

function navigateToDashboard(baseUrl: string) {
  // On mobile Tauri the bundled SPA only serves the connect wizard — once
  // paired we hop to the daemon-served dashboard. In a regular browser
  // (including desktop dev) the SPA we're running IS the dashboard, so
  // an internal hash-route change avoids a needless full reload onto a
  // different origin.
  if (isMobileTauri()) {
    window.location.href = baseUrl.replace(/\/$/, "") + "/dashboard";
  } else {
    window.location.hash = "#/overview";
  }
}

function defaultDisplayName(): string {
  if (/Android/.test(navigator.userAgent)) return "Android device";
  if (/iPad/.test(navigator.userAgent)) return "iPad";
  if (/iPhone|iPod/.test(navigator.userAgent)) return "iPhone";
  return "LibreFang Mobile";
}

function devicePlatform(): string {
  // Only label as ios when the UA actually identifies as iOS — defaulting
  // every non-Android client to "ios" pollutes the paired-device list when
  // the wizard is opened from a desktop browser for debugging.
  if (/Android/.test(navigator.userAgent)) return "android";
  if (/iPhone|iPad|iPod/.test(navigator.userAgent)) return "ios";
  return "unknown";
}

interface PairingPayload {
  v: number;
  base_url: string;
  token: string;
  expires_at: string;
}

function decodeQrPayload(raw: string): PairingPayload {
  // Parse librefang://pair?payload=<base64url-no-pad>
  const uri = new URL(raw);
  const payloadB64 = uri.searchParams.get("payload");
  if (!payloadB64) throw new Error("Invalid QR code: missing payload");

  // base64url (no-pad) → standard base64 → JSON. atob tolerates missing
  // padding in modern engines; explicit padEnd is not needed.
  const stdB64 = payloadB64.replace(/-/g, "+").replace(/_/g, "/");
  const payload = JSON.parse(atob(stdB64)) as PairingPayload;

  if (payload.v !== 1) throw new Error("Unsupported QR format version");
  if (new Date(payload.expires_at).getTime() < Date.now()) {
    throw new Error("QR code has expired — refresh it on the desktop");
  }
  if (
    !payload.base_url.startsWith("http://") &&
    !payload.base_url.startsWith("https://")
  ) {
    throw new Error("Invalid QR code: unexpected base_url protocol");
  }
  return payload;
}

export function ConnectWizardPage() {
  const [tab, setTab] = useState<Tab>("manual");
  const [step, setStep] = useState<Step>("idle");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [displayName, setDisplayName] = useState(defaultDisplayName());
  const [errorMsg, setErrorMsg] = useState("");

  const connectManual = useConnectManual();
  const connectQr = useConnectViaQr();

  // Already connected → verify creds still work, then skip wizard.
  // Stale creds (e.g. master key rotated) are cleared so the user lands
  // back here instead of getting stuck on a 401 in the dashboard.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const creds = await getCredentials();
      if (!creds || cancelled) return;
      try {
        const resp = await fetch(`${creds.base_url}/api/health`, {
          headers: { Authorization: `Bearer ${creds.api_key}` },
          signal: AbortSignal.timeout(5_000),
        });
        if (cancelled) return;
        if (resp.ok) {
          navigateToDashboard(creds.base_url);
        } else {
          await clearCredentials();
        }
      } catch {
        if (!cancelled) await clearCredentials();
      }
    })();
    return () => { cancelled = true; };
  }, []);

  function handleManualSubmit() {
    const url = baseUrl.trim().replace(/\/$/, "");
    const key = apiKey.trim();
    if (!url || !key) return;
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      setStep("error");
      setErrorMsg("URL must start with http:// or https://");
      return;
    }
    setStep("connecting");
    setErrorMsg("");
    connectManual.mutate(
      { baseUrl: url, apiKey: key },
      {
        onSuccess: () => {
          setStep("done");
          setTimeout(() => navigateToDashboard(url), 1200);
        },
        onError: (err: unknown) => {
          setStep("error");
          setErrorMsg(err instanceof Error ? err.message : "Connection failed");
        },
      },
    );
  }

  async function handleQrSubmit() {
    setStep("scanning");
    setErrorMsg("");
    try {
      const raw = await scanQrCode();
      if (!raw) {
        setStep("idle");
        return;
      }
      const payload = decodeQrPayload(raw);
      const pairingUrl = payload.base_url.replace(/\/$/, "");

      setStep("connecting");
      connectQr.mutate(
        {
          baseUrl: pairingUrl,
          token: payload.token,
          displayName: displayName.trim() || defaultDisplayName(),
          platform: devicePlatform(),
        },
        {
          onSuccess: (result) => {
            setStep("done");
            setTimeout(() => navigateToDashboard(result.baseUrl), 1200);
          },
          onError: (err: unknown) => {
            setStep("error");
            setErrorMsg(err instanceof Error ? err.message : "Pairing failed");
          },
        },
      );
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
              onClick={handleManualSubmit}
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
            <div className="space-y-1.5">
              <label htmlFor="device-name" className="text-xs font-semibold text-text-dim uppercase tracking-wider">
                Device name
              </label>
              <input
                id="device-name"
                type="text"
                placeholder="My iPhone"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                disabled={busy}
                className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-3 text-sm focus:border-brand focus:ring-2 focus:ring-brand/10 outline-none transition-colors placeholder:text-text-dim/40 disabled:opacity-50"
              />
              <p className="text-xs text-text-dim">
                Shown on the desktop so you can revoke this device later.
              </p>
            </div>
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
              onClick={() => void handleQrSubmit()}
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
