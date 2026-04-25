import { useEffect, useRef, useId, memo, type ReactNode } from "react";
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
  /** Container shape. `modal` (default) is centered with max-h-[90vh].
   *  `drawer-right` docks to the right edge at full viewport height — used
   *  for inspector workflows where the underlying list should stay visible
   *  for quick context-switching (Linear / Figma right panel pattern). */
  variant?: "modal" | "drawer-right";
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
export const Modal = memo(function Modal({
  isOpen,
  onClose,
  title,
  size = "md",
  hideCloseButton,
  disableBackdropClose,
  zIndex = 50,
  overflowVisible = false,
  variant = "modal",
  children,
}: ModalProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  const titleId = useId();
  const isDrawer = variant === "drawer-right";
  // Modal traps Tab inside the dialog (no escape from the focus loop).
  // Drawer leaves Tab free so keyboard users can hop back into the
  // underlying list (which is still interactive — see container's
  // pointer-events-none) without first hitting Esc.
  useFocusTrap(isOpen, dialogRef, true, !isDrawer);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCloseRef.current();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen]);

  if (!isOpen) return null;

  const handleBackdropClick = (e: React.MouseEvent) => {
    // Stop the click from bubbling to an ancestor backdrop.
    // `fixed inset-0` positions the overlay relative to the
    // viewport, but React synthetic events still follow the
    // DOM ancestor chain — so when this Modal is rendered
    // inside another backdrop-dismissable modal (e.g.
    // TomlViewer mounted inside HandsPage's HandDetailPanel),
    // closing this one via backdrop would otherwise also
    // close its parent. See codex review on #2722.
    e.stopPropagation();
    onClose();
  };

  // Drawer vs Modal differ in three ways:
  //   1. Position: drawer hugs the right edge full-height; modal centres.
  //   2. Dim: modal dims the page (focus on dialog); drawer leaves the
  //      page un-dimmed because the surrounding context — typically a
  //      list — should stay legible while the drawer inspects one item.
  //   3. Click-through: clicks outside the drawer panel pass through to
  //      the underlying page so users can pick another row in the list
  //      and the drawer updates in place (Linear / Figma inspector).
  //      The Modal still closes on backdrop click — that's its contract.
  const containerClass = isDrawer
    ? "fixed inset-0 flex items-stretch justify-end pointer-events-none"
    : "fixed inset-0 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-sm p-0 sm:p-4";
  const dialogClass = isDrawer
    ? `pointer-events-auto relative w-full ${SIZE_CLASSES[size]} h-full sm:rounded-l-2xl sm:border-l border-border-subtle bg-surface shadow-2xl animate-slide-in-right ${overflowVisible ? "overflow-visible" : "overflow-hidden"} flex flex-col`
    : `relative w-full ${SIZE_CLASSES[size]} rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl animate-fade-in-scale max-h-[90vh] ${overflowVisible ? "overflow-visible" : "overflow-hidden"} flex flex-col`;

  return (
    <div
      className={containerClass}
      style={{ zIndex }}
      // Backdrop dismissal is a modal contract; the drawer relies on Esc
      // and its explicit close button instead, since "click outside to
      // close" would race with the list-click-to-switch interaction.
      onClick={isDrawer || disableBackdropClose ? undefined : handleBackdropClick}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal={isDrawer ? "false" : "true"}
        aria-labelledby={titleId}
        className={dialogClass}
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
        {/* `overscroll-contain` stops wheel events from chaining into the
            page once the dialog hits its top/bottom — the bug surfaces
            in the drawer variant (page is interactive behind the panel)
            but the centred modal benefits too: a long modal pinned over
            a long page used to scroll the page after the modal bottomed
            out, which feels like the modal "leaks" the gesture. */}
        <div className="flex-1 overflow-y-auto overscroll-contain scrollbar-thin">{children}</div>
      </div>
    </div>
  );
});
