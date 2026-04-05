import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../../lib/store";
import { CheckCircle, XCircle, AlertCircle, X } from "lucide-react";

export function ToastContainer() {
  const toasts = useUIStore((s) => s.toasts);
  const removeToast = useUIStore((s) => s.removeToast);

  return (
    <div
      className="fixed bottom-6 right-6 z-100 flex flex-col gap-2 pointer-events-none"
      aria-live="polite"
      aria-atomic="false"
    >
      {toasts.map((toast) => (
        <ToastItem key={toast.id} {...toast} onDismiss={() => removeToast(toast.id)} />
      ))}
    </div>
  );
}

function ToastItem({ message, type, onDismiss }: { message: string; type: "success" | "error" | "info"; onDismiss: () => void }) {
  const { t } = useTranslation();
  useEffect(() => {
    const timer = setTimeout(onDismiss, 3500);
    return () => clearTimeout(timer);
  }, [onDismiss]);

  const styles = {
    success: "border-success/30 bg-success/10 text-success",
    error: "border-error/30 bg-error/10 text-error",
    info: "border-brand/30 bg-brand/10 text-brand",
  };

  const icons = {
    success: <CheckCircle className="h-4 w-4 shrink-0" />,
    error: <XCircle className="h-4 w-4 shrink-0" />,
    info: <AlertCircle className="h-4 w-4 shrink-0" />,
  };

  // Errors get role=alert (assertive) — they interrupt the current announcement.
  // Non-errors use role=status (polite) — they wait until the screen reader is idle.
  return (
    <div
      className={`pointer-events-auto flex items-center gap-3 rounded-xl border px-4 py-3 shadow-lg animate-in slide-in-from-right-5 ${styles[type]}`}
      role={type === "error" ? "alert" : "status"}
    >
      {icons[type]}
      <span className="text-sm font-bold">{message}</span>
      <button
        onClick={onDismiss}
        className="ml-2 opacity-60 hover:opacity-100 transition-opacity"
        aria-label={t("common.close")}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
