import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Keyboard, X } from "lucide-react";
import { G_NAV_SHORTCUTS } from "../../lib/useKeyboardShortcuts";
import { useFocusTrap } from "../../lib/useFocusTrap";

interface ShortcutsHelpProps {
  isOpen: boolean;
  onClose: () => void;
}

const GENERAL_SHORTCUTS: Array<{ keys: string[]; label: string }> = [
  { keys: ["⌘", "K"], label: "Open command palette" },
  { keys: ["/"], label: "Focus search on current page" },
  { keys: ["n"], label: "Create new (agent / workflow / plugin, page-aware)" },
  { keys: ["?"], label: "Show this cheat sheet" },
  { keys: ["Esc"], label: "Close dialog / modal" },
];

export function ShortcutsHelp({ isOpen, onClose }: ShortcutsHelpProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(isOpen, dialogRef, { setAriaModal: true });

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const navEntries = Object.entries(G_NAV_SHORTCUTS);

  return (
    <div className="fixed inset-0 z-100 flex items-end sm:items-start justify-center sm:pt-[10vh] p-0 sm:p-4">
      <div className="fixed inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-help-title"
        className="relative w-full sm:max-w-2xl rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl overflow-hidden animate-fade-in-scale"
      >
        <div className="flex items-center justify-between border-b border-border-subtle px-5 py-4">
          <div className="flex items-center gap-2.5">
            <div className="h-8 w-8 rounded-xl bg-brand/10 flex items-center justify-center text-brand">
              <Keyboard className="h-4 w-4" />
            </div>
            <h2 id="shortcuts-help-title" className="text-sm font-black tracking-tight">Keyboard shortcuts</h2>
          </div>
          <button
            onClick={onClose}
            className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            aria-label={t("common.close")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="max-h-[70vh] overflow-y-auto p-5 scrollbar-thin">
          <section className="mb-6">
            <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60 mb-3">General</h3>
            <div className="space-y-2">
              {GENERAL_SHORTCUTS.map((s) => (
                <div key={s.label} className="flex items-center justify-between py-1">
                  <span className="text-xs text-text-dim">{s.label}</span>
                  <div className="flex items-center gap-1">
                    {s.keys.map((k, i) => (
                      <kbd
                        key={i}
                        className="inline-flex h-6 min-w-[24px] items-center justify-center rounded border border-border-subtle bg-main px-1.5 text-[10px] font-mono font-semibold text-text-dim"
                      >
                        {k}
                      </kbd>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </section>

          <section>
            <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60 mb-3">
              Navigate (press <kbd className="inline-flex h-5 items-center rounded border border-border-subtle bg-main px-1 font-mono text-[9px]">g</kbd> then…)
            </h3>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-2">
              {navEntries.map(([key, { label }]) => (
                <div key={key} className="flex items-center justify-between py-1">
                  <span className="text-xs text-text-dim">{label}</span>
                  <div className="flex items-center gap-1">
                    <kbd className="inline-flex h-6 min-w-[24px] items-center justify-center rounded border border-border-subtle bg-main px-1.5 text-[10px] font-mono font-semibold text-text-dim">
                      g
                    </kbd>
                    <kbd className="inline-flex h-6 min-w-[24px] items-center justify-center rounded border border-border-subtle bg-main px-1.5 text-[10px] font-mono font-semibold text-text-dim">
                      {key}
                    </kbd>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
