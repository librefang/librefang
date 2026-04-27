import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useDrawerStore, type DrawerSize } from "../../lib/drawerStore";

const DESKTOP_WIDTH: Record<DrawerSize, string> = {
  sm: "lg:w-[360px]",
  md: "lg:w-[480px]",
  lg: "lg:w-[640px]",
  xl: "lg:w-[800px]",
};

// Push-style global drawer. Renders as a flex sibling of the main column in
// App.tsx — its width animates from 0 → target so the main content shrinks
// instead of being overlaid (mirrors the left sidebar's collapse behaviour).
//
// Two presentations from the same store:
//   - lg+ : push slot (animated width)
//   - <lg : fullscreen overlay, since push doesn't fit on narrow viewports
export function PushDrawer() {
  const { t } = useTranslation();
  const isOpen = useDrawerStore((s) => s.isOpen);
  const content = useDrawerStore((s) => s.content);
  const closeDrawer = useDrawerStore((s) => s.closeDrawer);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeDrawer();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, closeDrawer]);

  const size: DrawerSize = content?.size ?? "md";
  const desktopWidth = isOpen ? DESKTOP_WIDTH[size] : "lg:w-0";

  const header = (
    <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle shrink-0">
      {content?.title ? (
        <h3 className="text-sm font-bold tracking-tight truncate">{content.title}</h3>
      ) : <span />}
      <button
        onClick={closeDrawer}
        className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
        aria-label={t("common.close", { defaultValue: "Close" })}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );

  return (
    <>
      {/* Desktop push slot — flex sibling of main column. The inner wrapper
          keeps a fixed min-width so content doesn't reflow as the outer
          width animates between 0 and target. */}
      <aside
        className={`hidden lg:flex shrink-0 ${desktopWidth} flex-col border-l border-border-subtle bg-surface overflow-hidden transition-[width] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]`}
        aria-hidden={!isOpen}
      >
        {content && (
          <div className={`flex flex-col h-full ${size === "sm" ? "min-w-[360px]" : size === "md" ? "min-w-[480px]" : size === "lg" ? "min-w-[640px]" : "min-w-[800px]"}`}>
            {header}
            <div className="flex-1 overflow-y-auto overscroll-contain scrollbar-thin">
              {content.body}
            </div>
          </div>
        )}
      </aside>

      {/* Mobile overlay fallback — push doesn't fit on narrow viewports, so
          fall back to a fullscreen sheet. Backdrop closes on click. */}
      {isOpen && content && (
        <div
          className="fixed inset-0 z-50 lg:hidden bg-black/40 backdrop-blur-sm flex items-stretch justify-end"
          onClick={closeDrawer}
        >
          <div
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
