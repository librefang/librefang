import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { AlertTriangle, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { useFocusTrap } from "../../lib/useFocusTrap";
import { isTopEscapeLayer, registerEscapeLayer } from "./Modal";
import { fadeInScale, APPLE_EASE } from "../../lib/motion";

/// The dialog's stacking context, and its rank in `Modal`'s shared Escape stack.
///
/// Must equal the `z-[150]` literal on the backdrop below. That one is spelled
/// out rather than interpolated because Tailwind resolves arbitrary values by
/// scanning source text, so a computed class name yields no rule at all.
const CONFIRM_DIALOG_Z_INDEX = 150;

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
  onConfirm: () => void | Promise<void>;
  onClose: () => void;
}

/// Modal confirmation dialog. Replaces `window.confirm()` with a styled
/// dialog that matches the rest of the dashboard, supports destructive
/// styling, and slides up from the bottom on mobile.
export const ConfirmDialog = React.memo(function ConfirmDialog({
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
  const isConfirmingRef = useRef(false);
  const activeConfirmationRef = useRef<object | null>(null);
  const [isConfirming, setIsConfirming] = useState(false);
  const escapeTokenRef = useRef<symbol | null>(null);
  const requestCloseRef = useRef<() => void>(() => {});
  const onCloseRef = useRef(onClose);
  const onConfirmRef = useRef(onConfirm);
  const titleId = useId();
  const messageId = useId();
  onCloseRef.current = onClose;
  onConfirmRef.current = onConfirm;
  useFocusTrap(isOpen, dialogRef, true);

  const requestClose = useCallback(() => {
    if (isConfirmingRef.current) return;
    onCloseRef.current();
  }, []);
  requestCloseRef.current = requestClose;

  const requestConfirm = useCallback(() => {
    if (isConfirmingRef.current) return;
    const confirmationToken = {};
    activeConfirmationRef.current = confirmationToken;
    isConfirmingRef.current = true;
    setIsConfirming(true);
    void (async () => {
      try {
        await onConfirmRef.current();
        if (activeConfirmationRef.current !== confirmationToken) return;
        onCloseRef.current();
      } catch {
        if (activeConfirmationRef.current !== confirmationToken) return;
        isConfirmingRef.current = false;
        setIsConfirming(false);
      }
    })();
  }, []);

  useEffect(() => {
    activeConfirmationRef.current = null;
    isConfirmingRef.current = false;
    setIsConfirming(false);
    return () => {
      activeConfirmationRef.current = null;
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [isOpen]);

  // Escape goes through `Modal`'s shared stack rather than through a listener
  // of this component's own.
  //
  // Both used to be `window` `keydown` handlers, and `handleModalEscape` calls
  // `stopImmediatePropagation`, so the one registered first won the key
  // outright. A dialog opened over a modal therefore closed the modal — the
  // layer underneath — and unmounted the dialog with it, dropping the user out
  // of the context they were working in (#8336). Registering here means the
  // topmost layer wins by z-index, not by mount order.
  //
  // `z-[150]` on the backdrop below is the number this must agree with;
  // `Modal`'s default is 50.
  useEffect(() => {
    if (!isOpen) return;
    const token = Symbol("confirm-dialog");
    escapeTokenRef.current = token;
    const unregister = registerEscapeLayer({
      token,
      zIndex: CONFIRM_DIALOG_Z_INDEX,
      close: () => requestCloseRef.current(),
    });
    return () => {
      escapeTokenRef.current = null;
      unregister();
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    // Enter inside a textarea/input/contenteditable must NOT trigger confirm —
    // users routinely have an editable field focused while a dialog is open
    // (#3389).
    const isEditableTarget = (target: EventTarget | null): boolean => {
      if (!(target instanceof HTMLElement)) return false;
      const tag = target.tagName;
      if (tag === "TEXTAREA" || tag === "INPUT" || tag === "SELECT") return true;
      if (target.isContentEditable) return true;
      return false;
    };
    const handleKey = (e: KeyboardEvent) => {
      // Enter confirms — safer on non-destructive dialogs; for destructive
      // we still require a click so users can't accidentally nuke data.
      //
      // Gated on being the topmost layer for the same reason Escape is: a
      // dialog with something stacked above it is not the thing the user is
      // typing into, and confirming it from underneath is the same reach-through
      // bug one key over.
      if (e.key === "Enter" && tone !== "destructive" && !isEditableTarget(e.target)) {
        const token = escapeTokenRef.current;
        if (!token || !isTopEscapeLayer(token)) return;
        e.preventDefault();
        requestConfirm();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, requestConfirm, tone]);

  const isDestructive = tone === "destructive";
  const confirmBtnClass = isDestructive
    ? "bg-error text-white hover:bg-error/90 shadow-lg shadow-error/20"
    : "bg-brand text-white hover:bg-brand/90 shadow-lg shadow-brand/20";

  return (
    <AnimatePresence>
      {isOpen && (
        <motion.div
          // Keep the literal in step with `CONFIRM_DIALOG_Z_INDEX` above. It cannot be
          // interpolated: Tailwind's scanner reads class strings statically, so a template
          // literal would produce no `z-index` rule at all and the dialog would render under
          // the modal it is supposed to sit on.
          className="fixed inset-0 z-[150] flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm p-0 sm:p-4"
          onClick={requestClose}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18, ease: APPLE_EASE }}
        >
      <motion.div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={messageId}
        className="relative w-full sm:max-w-md rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        variants={fadeInScale}
        initial="initial"
        animate="animate"
        exit="exit"
      >
        <button
          onClick={requestClose}
          disabled={isConfirming}
          className="absolute right-3 top-3 h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors disabled:cursor-not-allowed disabled:opacity-50"
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
            <h3 id={titleId} className="text-sm font-black tracking-tight">{title}</h3>
            <p id={messageId} className="mt-1.5 text-xs text-text-dim leading-relaxed">{message}</p>
          </div>
        </div>
        <div className="flex gap-2 border-t border-border-subtle/50 px-5 py-3">
          <button
            onClick={requestClose}
            disabled={isConfirming}
            className="flex-1 rounded-xl border border-border-subtle bg-surface py-2.5 text-xs font-bold text-text-dim hover:bg-surface-hover transition-colors disabled:cursor-not-allowed disabled:opacity-50"
          >
            {cancelLabel ?? t("common.cancel", { defaultValue: "Cancel" })}
          </button>
          <button
            onClick={requestConfirm}
            disabled={isConfirming}
            aria-busy={isConfirming}
            className={`flex-1 rounded-xl py-2.5 text-xs font-bold transition-all hover:-translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:translate-y-0 ${confirmBtnClass}`}
          >
            {confirmLabel ?? t("common.confirm", { defaultValue: "Confirm" })}
          </button>
        </div>
      </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
});
