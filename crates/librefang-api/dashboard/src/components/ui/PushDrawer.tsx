import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useDrawerStore, type DrawerSize } from "../../lib/drawerStore";
import { useFocusTrap } from "../../lib/useFocusTrap";

const DESKTOP_WIDTH: Record<DrawerSize, string> = {
  sm: "lg:w-[360px]",
  md: "lg:w-[480px]",
  lg: "lg:w-[640px]",
  xl: "lg:w-[720px]",
  "2xl": "lg:w-[800px]",
  "3xl": "lg:w-[960px]",
  "4xl": "lg:w-[1100px]",
  "5xl": "lg:w-[1280px]",
};

const MIN_WIDTH: Record<DrawerSize, string> = {
  sm: "lg:min-w-[360px]",
  md: "lg:min-w-[480px]",
  lg: "lg:min-w-[640px]",
  xl: "lg:min-w-[720px]",
  "2xl": "lg:min-w-[800px]",
  "3xl": "lg:min-w-[960px]",
  "4xl": "lg:min-w-[1100px]",
  "5xl": "lg:min-w-[1280px]",
};

// Mirrors the `--breakpoint-lg: 1000px` override in index.css (#4873) so
// JS- and CSS-driven layout decisions never disagree at the iPad portrait
// boundary. Kept in px (not rem) deliberately: `window.matchMedia` does
// not scale with the root font-size, so a px-vs-rem mix between CSS
// (`lg:` variant) and JS (this hook) would diverge under iOS text-zoom.
// If you change one, change the other.
const MOBILE_QUERY = "(max-width: 999px)";

function readIsMobile(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia(MOBILE_QUERY).matches;
}

function useIsMobile() {
  // Lazy-init from matchMedia at first render so we don't flash a desktop
  // focus-trap target on a phone for one frame before the effect runs.
  const [isMobile, setIsMobile] = useState(readIsMobile);
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mql = window.matchMedia(MOBILE_QUERY);
    setIsMobile(mql.matches);
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mql.addEventListener("change", handler);
    return () => mql.removeEventListener("change", handler);
  }, []);
  return isMobile;
}

// Push-style global drawer host. Renders as a flex sibling of the main
// column in App.tsx — its width animates from 0 → target so the main
// content shrinks to make room (mirrors the left sidebar's collapse).
//
// Two presentations from the same store:
//   - lg+ : push slot, animated width
//   - <lg : fullscreen overlay sheet, since push doesn't fit on narrow
//           viewports. Backdrop click + Esc both dismiss.
export function PushDrawer() {
  const { t } = useTranslation();
  const isOpen = useDrawerStore((s) => s.isOpen);
  const content = useDrawerStore((s) => s.content);
  const close = useDrawerStore((s) => s.close);

  const desktopRef = useRef<HTMLDivElement>(null);
  const mobileRef = useRef<HTMLDivElement>(null);

  // Scope each focus trap to the viewport actually showing it. Without the
  // `!isMobile` / `isMobile` split, both traps would activate on every
  // viewport — wasted work, and also a subtle a11y trap if either
  // `<aside>` (desktop) or the mobile overlay <div> were visible
  // simultaneously while the other is `display:none`.
  const isMobile = useIsMobile();
  useFocusTrap(isOpen && !isMobile, desktopRef, false, false);
  useFocusTrap(isOpen && isMobile, mobileRef, false);

  // Single dismissal path — DrawerPanel observes the store flip and calls
  // its own onClose to keep parent state in sync. We don't fire content
  // .onClose here directly; the store-flip-watcher in DrawerPanel does
  // that, so closing twice (e.g. Esc then click X mid-frame) doesn't
  // double-fire the callback.
  const triggerClose = useCallback(() => close(), [close]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key !== "Escape") return;
      const target = e.target;
      if (!(target instanceof HTMLElement)) return;
      if (target.closest("[role='dialog']") && !target.closest("[role='dialog'][data-drawer-root]")) return;
      e.stopImmediatePropagation();
      triggerClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, triggerClose]);

  const size: DrawerSize = content?.size ?? "md";
  const desktopWidth = isOpen ? DESKTOP_WIDTH[size] : "lg:w-0";
  const minWidth = MIN_WIDTH[size];

  const header = !content?.hideCloseButton ? (
    <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle shrink-0">
      {content?.title ? (
        <h3 className="text-sm font-bold tracking-tight truncate">{content.title}</h3>
      ) : <span />}
      <button
        onClick={triggerClose}
        className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
        aria-label={t("common.close", { defaultValue: "Close" })}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  ) : null;

  return (
    <>
      {/* Desktop push slot. The aside owns the collapsing width; the inner
          wrapper has a min-width so content doesn't reflow as the outer
          width animates between 0 and target. */}
      <aside
        ref={desktopRef}
        className={`hidden lg:flex shrink-0 ${desktopWidth} flex-col border-l border-border-subtle bg-surface overflow-hidden transition-[width] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]`}
        aria-hidden={!isOpen}
      >
        {content && (
          <div className={`flex flex-col h-full ${minWidth}`}>
            {header}
            <div className="flex-1 overflow-y-auto overscroll-contain scrollbar-thin">
              {content.body}
            </div>
          </div>
        )}
      </aside>

      {/* Mobile overlay fallback. Backdrop closes on click.
          z-index: must sit above the slide-in sidebar (z-50 in App.tsx) so
          the close button stays tappable when both end up in mobile mode at
          the same time (#4873). Stay BELOW the OfflineBanner (z-[60]) — the
          banner is a daemon-disconnect signal that must remain visible even
          when a drawer is open — and below dropdown menus (z-[90]/z-[100]).
          z-[55] threads the needle. */}
      {isOpen && content && (
        <div
          className="fixed inset-0 z-[55] lg:hidden bg-black/40 backdrop-blur-sm flex items-stretch justify-end"
          onClick={triggerClose}
          role="dialog"
          aria-modal="true"
          data-drawer-root
        >
          <div
            ref={mobileRef}
            className="w-full bg-surface flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            {header}
            <div className="flex-1 overflow-y-auto overscroll-contain scrollbar-thin">
              {content.body}
            </div>
          </div>
        </div>
      )}
    </>
  );
}
