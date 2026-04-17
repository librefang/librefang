import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useFocusTrap } from "../../lib/useFocusTrap";

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title?: string;
  /** Width cap. Defaults to "md" (max-w-md). */
  size?: "sm" | "md" | "lg" | "xl" | "2xl" | "3xl" | "4xl" | "5xl";
  /** Hide the default close X button (e.g. if the body supplies its own). */
  hideCloseButton?: boolean;
  /** Disable close-on-backdrop-click (destructive flows). */
  disableBackdropClose?: boolean;
  /** z-index override — defaults to 50. */
  zIndex?: number;
  /** Allow content to overflow the modal container (e.g. for cmdk dropdowns). Defaults to false. */
  overflowVisible?: boolean;
  children: ReactNode;
}

const SIZE_CLASSES: Record<NonNullable<ModalProps["size"]>, string> = {
  sm: "sm:max-w-sm",
  md: "sm:max-w-md",
  lg: "sm:max-w-lg",
  xl: "sm:max-w-xl",
  "2xl": "sm:max-w-2xl",
  "3xl": "sm:max-w-3xl",
  "4xl": "sm:max-w-4xl",
  "5xl": "sm:max-w-5xl",
};

/// Shared modal shell. Handles the cross-cutting concerns every page
/// modal needs:
///
/// - Backdrop + click-to-dismiss (unless `disableBackdropClose`)
/// - Escape key closes
/// - Bottom-sheet on <640px, centered on sm+
/// - Focus trap (Tab cycles inside, Shift+Tab reverses)
/// - Focus restoration on close
/// - aria-modal + role="dialog" for screen readers
///
/// Children render inside the dialog container — provide your own
/// body content and (optionally) your own header/footer.
export function Modal({
  isOpen,
  onClose,
  title,
  size = "md",
  hideCloseButton,
  disableBackdropClose,
  zIndex = 50,
  overflowVisible = false,
  children,
}: ModalProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(isOpen, dialogRef);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const titleId = title ? "modal-title-" + Math.random().toString(36).slice(2, 8) : undefined;

  return (
    <div
      className="fixed inset-0 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-sm p-0 sm:p-4"
      style={{ zIndex }}
      onClick={disableBackdropClose ? undefined : onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={`relative w-full ${SIZE_CLASSES[size]} rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl animate-fade-in-scale max-h-[90vh] ${overflowVisible ? "overflow-visible" : "overflow-hidden"} flex flex-col`}
        onClick={(e) => e.stopPropagation()}
      >
        {(title || !hideCloseButton) && (
          <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle shrink-0">
            {title ? (
              <h3 id={titleId} className="text-sm font-bold tracking-tight">{title}</h3>
            ) : <span />}
            {!hideCloseButton && (
              <button
                onClick={onClose}
                className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                aria-label={t("common.close", { defaultValue: "Close" })}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        )}
        <div className="flex-1 overflow-y-auto scrollbar-thin">{children}</div>
      </div>
    </div>
  );
}
