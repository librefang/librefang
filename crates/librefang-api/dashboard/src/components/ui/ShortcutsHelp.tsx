import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Keyboard, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { G_NAV_SHORTCUTS } from "../../lib/useKeyboardShortcuts";
import { useFocusTrap } from "../../lib/useFocusTrap";
import { fadeInScale, APPLE_EASE } from "../../lib/motion";

interface ShortcutsHelpProps {
  isOpen: boolean;
  onClose: () => void;
}

const GENERAL_SHORTCUTS: Array<{ keys: string[]; labelKey: string }> = [
  { keys: ["⌘", "K"], labelKey: "cmd_k" },
  { keys: ["/"], labelKey: "focus_search" },
  { keys: ["n"], labelKey: "create_new" },
  { keys: ["?"], labelKey: "show_help" },
  { keys: ["Esc"], labelKey: "close_dialog" },
];

const NAV_ENTRIES = Object.entries(G_NAV_SHORTCUTS);

const KBD_CLASS =
  "inline-flex h-6 min-w-[24px] items-center justify-center rounded border border-border-subtle bg-main px-1.5 text-[10px] font-mono font-semibold text-text-dim";

export function ShortcutsHelp({ isOpen, onClose }: ShortcutsHelpProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(isOpen, dialogRef, true);

  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCloseRef.current();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isOpen]);

  return (
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-100 flex items-end sm:items-start justify-center sm:pt-[10vh] p-0 sm:p-4">
          <motion.div
            className="fixed inset-0 bg-black/60 backdrop-blur-sm"
            aria-hidden="true"
            onClick={onClose}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18, ease: APPLE_EASE }}
          />
          <motion.div
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="shortcuts-help-title"
            className="relative w-full sm:max-w-2xl rounded-t-2xl sm:rounded-2xl border border-border-subtle bg-surface shadow-2xl overflow-hidden"
            variants={fadeInScale}
            initial="initial"
            animate="animate"
            exit="exit"
          >
        <div className="flex items-center justify-between border-b border-border-subtle px-5 py-4">
          <div className="flex items-center gap-2.5">
            <div className="h-8 w-8 rounded-xl bg-primary/10 flex items-center justify-center text-primary">
              <Keyboard className="h-4 w-4" />
            </div>
            <h2 id="shortcuts-help-title" className="text-sm font-black tracking-tight">{t("shortcuts_help.title")}</h2>
          </div>
          <button
            onClick={onClose}
            className="h-7 w-7 flex items-center justify-center rounded-lg text-text-dim hover:text-primary hover:bg-surface-hover transition-colors"
            aria-label={t("common.close")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="max-h-[70vh] overflow-y-auto p-5 scrollbar-thin">
          <section className="mb-6">
            <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/60 mb-3">{t("shortcuts_help.general_heading")}</h3>
            <div className="space-y-2">
              {GENERAL_SHORTCUTS.map((s) => (
                <div key={s.labelKey} className="flex items-center justify-between py-1">
                  <span className="text-xs text-text-dim">{t(`shortcuts_help.general.${s.labelKey}`)}</span>
                  <div className="flex items-center gap-1">
                    {s.keys.map((k, i) => (
                      <kbd key={i} className={KBD_CLASS}>
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
              {t("shortcuts_help.nav_heading_prefix")} <kbd className="inline-flex h-5 items-center rounded border border-border-subtle bg-main px-1 font-mono text-[9px]">g</kbd> {t("shortcuts_help.nav_heading_suffix")}
            </h3>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-2">
              {NAV_ENTRIES.map(([key, { labelKey }]) => (
                <div key={key} className="flex items-center justify-between py-1">
                  <span className="text-xs text-text-dim">{t(`shortcuts_help.nav.${labelKey}`)}</span>
                  <div className="flex items-center gap-1">
                    <kbd className={KBD_CLASS}>
                      g
                    </kbd>
                    <kbd className={KBD_CLASS}>
                      {key}
                    </kbd>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
