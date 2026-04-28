import { useEffect, useRef } from "react";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import QRCode from "qrcode";
import { Smartphone, RefreshCw, CheckCircle, Clock, Trash2 } from "lucide-react";
import { usePairingRequest, usePairedDevices, useRemovePairedDevice } from "../lib/queries/pairing";
import { pairingKeys } from "../lib/queries/keys";

function QRCanvas({ uri }: { uri: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (canvasRef.current) {
      QRCode.toCanvas(canvasRef.current, uri, {
        width: 240,
        margin: 2,
        color: { dark: "#0f172a", light: "#ffffff" },
      });
    }
  }, [uri]);
  return <canvas ref={canvasRef} className="rounded-xl" />;
}

function CountdownBadge({ expiresAt }: { expiresAt: string }) {
  const [secs, setSecs] = useState(() =>
    Math.max(0, Math.round((new Date(expiresAt).getTime() - Date.now()) / 1000)),
  );
  useEffect(() => {
    const id = setInterval(() => setSecs((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(id);
  }, [expiresAt]);
  const mins = Math.floor(secs / 60);
  const s = secs % 60;
  const expired = secs === 0;
  return (
    <span
      className={`flex items-center gap-1.5 text-sm font-mono ${expired ? "text-error" : "text-text-dim"}`}
    >
      <Clock className="w-4 h-4" />
      {expired ? "Expired" : `${mins}:${String(s).padStart(2, "0")}`}
    </span>
  );
}

export function MobilePairingPage() {
  const qc = useQueryClient();
  const { data: req, error, isLoading, refetch } = usePairingRequest(true);
  const { data: devices = [] } = usePairedDevices();
  const removeDevice = useRemovePairedDevice();

  const expired = req ? new Date(req.expires_at).getTime() < Date.now() : false;

  const refresh = () => {
    qc.removeQueries({ queryKey: pairingKeys.request() });
    refetch();
  };

  if (error) {
    const isDisabled = (error as { status?: number })?.status === 404;
    return (
      <div className="max-w-xl mx-auto px-4 py-12 text-center space-y-3">
        <Smartphone className="w-10 h-10 mx-auto text-text-dim" />
        <p className="font-semibold">
          {isDisabled ? "Device pairing is disabled" : "Failed to generate pairing code"}
        </p>
        {isDisabled ? (
          <p className="text-sm text-text-dim">
            Enable pairing in{" "}
            <a href="/dashboard/config/security" className="text-brand underline">
              Config → Security
            </a>{" "}
            (<code className="text-xs">pairing.enabled = true</code>).
          </p>
        ) : (
          <button
            onClick={refresh}
            className="rounded-xl bg-brand px-4 py-2 text-sm text-white font-medium"
          >
            Try again
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="max-w-2xl mx-auto px-4 py-8 space-y-8">
      {/* Header */}
      <div className="space-y-1">
        <h1 className="text-xl font-bold flex items-center gap-2">
          <Smartphone className="w-6 h-6 text-brand" />
          Mobile Pairing
        </h1>
        <p className="text-sm text-text-dim">
          Open the LibreFang mobile app and tap <strong>Scan QR</strong> to connect to this
          daemon.
        </p>
      </div>

      {/* QR Card */}
      <div className="rounded-2xl border border-border-subtle bg-surface p-6 flex flex-col items-center gap-4">
        {isLoading ? (
          <div className="w-60 h-60 flex items-center justify-center">
            <RefreshCw className="w-8 h-8 text-text-dim animate-spin" />
          </div>
        ) : req ? (
          <>
            <div className={expired ? "opacity-30 pointer-events-none" : ""}>
              <QRCanvas uri={req.qr_uri} />
            </div>
            <div className="flex items-center gap-4">
              <CountdownBadge expiresAt={req.expires_at} />
              <button
                onClick={refresh}
                className="flex items-center gap-1.5 text-sm text-brand hover:underline"
              >
                <RefreshCw className="w-3.5 h-3.5" />
                Refresh
              </button>
            </div>
            {expired && (
              <p className="text-sm text-error">QR code expired — refresh to get a new one.</p>
            )}
          </>
        ) : null}
      </div>

      {/* Paired Devices */}
      {devices.length > 0 && (
        <div className="space-y-3">
          <h2 className="text-sm font-bold uppercase tracking-wider text-text-dim">
            Paired Devices
          </h2>
          <div className="space-y-2">
            {devices.map((d) => (
              <div
                key={d.device_id}
                className="flex items-center justify-between rounded-xl border border-border-subtle bg-surface p-3"
              >
                <div className="flex items-center gap-3">
                  <CheckCircle className="w-4 h-4 text-success shrink-0" />
                  <div>
                    <p className="text-sm font-medium">{d.display_name}</p>
                    <p className="text-xs text-text-dim">
                      {d.platform} · paired {new Date(d.paired_at).toLocaleDateString()}
                    </p>
                  </div>
                </div>
                <button
                  onClick={() => removeDevice.mutate(d.device_id)}
                  disabled={removeDevice.isPending}
                  className="rounded-lg p-1.5 text-text-dim hover:text-error transition-colors"
                  title="Remove device"
                >
                  <Trash2 className="w-4 h-4" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
