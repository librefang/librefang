import { memo, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { useUIStore } from "../../lib/store";
import { CheckCircle, XCircle, AlertCircle, X } from "lucide-react";
import { toastSlide } from "../../lib/motion";

const TOAST_STYLES: Record<"success" | "error" | "info", string> = {
  success: "border-success/30 bg-success/10 text-success",
  error: "border-error/30 bg-error/10 text-error",
  info: "border-brand/30 bg-brand/10 text-brand",
};

const TOAST_ICONS: Record<"success" | "error" | "info", React.ReactNode> = {
  success: <CheckCircle className="h-4 w-4 shrink-0" />,
  error: <XCircle className="h-4 w-4 shrink-0" />,
  info: <AlertCircle className="h-4 w-4 shrink-0" />,
};

export function ToastContainer() {
  const toasts = useUIStore((s) => s.toasts);
  const removeToast = useUIStore((s) => s.removeToast);

  return (
    <div
      className="fixed bottom-6 right-6 z-100 flex flex-col gap-2 pointer-events-none"
      aria-live="polite"
      aria-atomic="false"
    >
      <AnimatePresence>
        {toasts.map((toast) => (
          <ToastItem key={toast.id} id={toast.id} message={toast.message} type={toast.type} removeToast={removeToast} />
        ))}
      </AnimatePresence>
    </div>
  );
}

const ToastItem = memo(function ToastItem({ id, message, type, removeToast }: { id: string; message: string; type: "success" | "error" | "info"; removeToast: (id: string) => void }) {
  const { t } = useTranslation();

  const onDismiss = useCallback(() => removeToast(id), [id, removeToast]);

  useEffect(() => {
    const timer = setTimeout(onDismiss, 3500);
    return () => clearTimeout(timer);
  }, [onDismiss]);

  // Errors get role=alert (assertive) — they interrupt the current announcement.
  // Non-errors use role=status (polite) — they wait until the screen reader is idle.
  return (
    <motion.div
      layout
      variants={toastSlide}
      initial="initial"
      animate="animate"
      exit="exit"
      className={`pointer-events-auto flex items-center gap-3 rounded-xl border px-4 py-3 shadow-lg ${TOAST_STYLES[type]}`}
      role={type === "error" ? "alert" : "status"}
    >
      {TOAST_ICONS[type]}
      <span className="text-sm font-bold">{message}</span>
      <button
        onClick={onDismiss}
        className="ml-2 opacity-60 hover:opacity-100 transition-opacity"
        aria-label={t("common.close")}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </motion.div>
  );
});
