import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import QRCode from "qrcode";
import { Smartphone, RefreshCw, CheckCircle, Clock, Trash2, AlertCircle } from "lucide-react";
import { usePairingRequest, usePairedDevices, useRemovePairedDevice } from "../lib/queries/pairing";
import { ApiError } from "../lib/http/errors";
import { pairingKeys } from "../lib/queries/keys";

function QRCanvas({ uri }: { uri: string }) {
  const { t } = useTranslation();
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
  return (
    <canvas
      ref={canvasRef}
      role="img"
      aria-label={t("mobile_pairing.qr_aria_label")}
      className="rounded-xl"
    />
  );
}

function CountdownBadge({ expiresAt }: { expiresAt: string }) {
  const { t } = useTranslation();
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
      {expired ? t("mobile_pairing.expired_label") : `${mins}:${String(s).padStart(2, "0")}`}
    </span>
  );
}

export function MobilePairingPage() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const { data: req, error, isLoading, refetch } = usePairingRequest(true);
  const { data: devices = [] } = usePairedDevices();
  const removeDevice = useRemovePairedDevice();

  const expired = req ? new Date(req.expires_at).getTime() < Date.now() : false;
  // Translator-supplied markup only (`<strong>`); no user input is interpolated
  // into these strings, so dangerouslySetInnerHTML is safe here.
  const subtitleHtml = { __html: t("mobile_pairing.subtitle") };
  const disabledBodyHtml = {
    // i18next <link> pseudo-tag → real <a>. Source is translator-only markup;
    // no user input flows through these strings, so innerHTML is safe.
    __html: t("mobile_pairing.error_disabled_body").replace(
      /<link>(.*?)<\/link>/s,
      '<a href="/dashboard/config/security" class="text-brand underline">$1</a>',
    ),
  };

  const refresh = () => {
    qc.removeQueries({ queryKey: pairingKeys.request() });
    refetch();
  };

  if (error) {
    const isDisabled = error instanceof ApiError && error.status === 404;
    return (
      <div className="max-w-xl mx-auto px-4 py-12 text-center space-y-3">
        <Smartphone className="w-10 h-10 mx-auto text-text-dim" />
        <p className="font-semibold">
          {isDisabled
            ? t("mobile_pairing.error_disabled_title")
            : t("mobile_pairing.error_generic_title")}
        </p>
        {isDisabled ? (
          <p className="text-sm text-text-dim" dangerouslySetInnerHTML={disabledBodyHtml} />
        ) : (
          <button
            onClick={refresh}
            className="rounded-xl bg-brand px-4 py-2 text-sm text-white font-medium"
          >
            {t("mobile_pairing.btn_try_again")}
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
          {t("mobile_pairing.title")}
        </h1>
        <p className="text-sm text-text-dim" dangerouslySetInnerHTML={subtitleHtml} />
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
                {t("mobile_pairing.refresh")}
              </button>
            </div>
            {expired && (
              <p className="text-sm text-error">{t("mobile_pairing.expired_message")}</p>
            )}
          </>
        ) : null}
      </div>

      {/* Paired Devices */}
      {devices.length > 0 && (
        <div className="space-y-3">
          <h2 className="text-sm font-bold uppercase tracking-wider text-text-dim">
            {t("mobile_pairing.paired_devices_heading")}
          </h2>
          {removeDevice.isError && (
            <div className="flex items-center gap-2 rounded-lg border border-error/20 bg-error/5 p-2.5 text-sm text-error">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>
                {t("mobile_pairing.remove_failed", {
                  reason:
                    removeDevice.error instanceof Error
                      ? removeDevice.error.message
                      : t("mobile_pairing.remove_unknown_error"),
                })}
              </span>
            </div>
          )}
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
                      {t("mobile_pairing.paired_at", {
                        platform: d.platform,
                        date: new Date(d.paired_at).toLocaleDateString(),
                      })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={() => removeDevice.mutate(d.device_id)}
                  disabled={removeDevice.isPending}
                  className="rounded-lg p-1.5 text-text-dim hover:text-error transition-colors"
                  title={t("mobile_pairing.remove_title")}
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
