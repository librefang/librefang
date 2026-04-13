import { useEffect, useRef } from "react";
import { AlertTriangle, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useFocusTrap } from "../../lib/useFocusTrap";

interface ConfirmDialogProps {
  isOpen: boolean;
  title: string;
  message: string;
  /** Label for the confirm button. Defaults to the translated "confirm". */
  confirmLabel?: string;
  /** Label for the cancel button. Defaults to the translated "cancel". */
  cancelLabel?: string;
  /** Visual tone — destructive renders the confirm button in error colors. */
  tone?: "default" | "destructive";
  onConfirm: () => void;
  onClose: () => void;
}

/// Modal confirmation dialog. Replaces `window.confirm()` with a styled
/// dialog that matches the rest of the dashboard, supports destructive
/// styling, and slides up from the bottom on mobile.
export function ConfirmDialog({
  isOpen,
  title,
  message,
  confirmLabel,
  cancelLabel,
  tone = "default",
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(isOpen, dialogRef);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      // Enter confirms — safer on non-destructive dialogs; for destructive
      // we still require a click so users can't accidentally nuke data.
      if (e.key === "Enter" && tone !== "destructive") onConfirm();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, onClose, onConfirm, tone]);

  if (!isOpen) return null;

  const isDestructive = tone === "destructive";
  const confirmBtnClass = isDestructive
    ? "bg-error text-white hover:bg-error/90 shadow-lg shadow-error/20"
    : "bg-brand text-white hover:bg-brand/90 shadow-lg shadow-brand/20";

  return (
    <div
      className="fixed inset-0 z-[150] flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm p-0 sm:p-4"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        className="relative w-full sm:max-w-md rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl animate-fade-in-scale"
        onClick={(e) => e.stopPropagation()}
      >
        <button
          onClick={onClose}
          className="absolute right-3 top-3 h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
          aria-label={t("common.close", { defaultValue: "Close" })}
        >
          <X className="h-3.5 w-3.5" />
        </button>
        <div className="flex items-start gap-4 p-5 pr-12">
          <div
            className={`h-10 w-10 shrink-0 rounded-xl flex items-center justify-center ${
              isDestructive ? "bg-error/10 text-error" : "bg-brand/10 text-brand"
            }`}
          >
            <AlertTriangle className="h-5 w-5" />
          </div>
          <div className="flex-1 min-w-0">
            <h3 id="confirm-dialog-title" className="text-sm font-black tracking-tight">{title}</h3>
            <p className="mt-1.5 text-xs text-text-dim leading-relaxed">{message}</p>
          </div>
        </div>
        <div className="flex gap-2 border-t border-border-subtle/50 px-5 py-3">
          <button
            onClick={onClose}
            className="flex-1 rounded-xl border border-border-subtle bg-surface py-2.5 text-xs font-bold text-text-dim hover:bg-surface-hover transition-colors"
          >
            {cancelLabel ?? t("common.cancel")}
          </button>
          <button
            onClick={() => {
              onConfirm();
              onClose();
            }}
            className={`flex-1 rounded-xl py-2.5 text-xs font-bold transition-all hover:-translate-y-0.5 ${confirmBtnClass}`}
          >
            {confirmLabel ?? t("common.confirm", { defaultValue: "Confirm" })}
          </button>
        </div>
      </div>
    </div>
  );
}
