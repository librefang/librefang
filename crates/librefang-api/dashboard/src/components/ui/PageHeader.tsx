import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, HelpCircle } from "lucide-react";
import { Modal } from "./Modal";

interface PageHeaderProps {
  icon: ReactNode;
  title: string;
  subtitle?: string;
  /** Optional small label rendered next to the title (e.g. "Beta", count). */
  badge?: string;
  actions?: ReactNode;
  isFetching?: boolean;
  onRefresh?: () => void;
  helpText?: string;
}

export function PageHeader({ icon, title, subtitle, badge, actions, isFetching, onRefresh, helpText }: PageHeaderProps) {
  const { t } = useTranslation();
  const [showHelp, setShowHelp] = useState(false);

  return (
    <>
      <header className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div className="p-1.5 rounded-lg bg-brand/10 text-brand shrink-0">{icon}</div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h1 className="text-base font-extrabold tracking-tight">{title}</h1>
              {badge && (
                <span className="inline-flex items-center rounded-md border border-border-subtle bg-main/40 px-1.5 py-0.5 text-[10px] font-semibold text-text-dim">
                  {badge}
                </span>
              )}
            </div>
            {subtitle && <p className="text-[11px] text-text-dim hidden sm:block">{subtitle}</p>}
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0 flex-wrap justify-end">
          {actions}
          {helpText && (
            <button
              onClick={() => setShowHelp(true)}
              className="flex h-8 w-8 items-center justify-center rounded-xl border border-border-subtle bg-surface text-text-dim hover:text-brand hover:border-brand/30 transition-colors duration-200"
              title={t("common.help", { defaultValue: "Help" })}
              aria-label={t("common.help", { defaultValue: "Help" })}
            >
              <HelpCircle className="h-4 w-4" />
            </button>
          )}
          {onRefresh && (
            <button
              className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 hover:shadow-sm transition-colors duration-200"
              onClick={onRefresh}
              aria-label={t("common.refresh")}
              aria-busy={isFetching}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isFetching ? "animate-spin motion-reduce:animate-none" : ""}`} />
              <span className="hidden sm:inline">{t("common.refresh")}</span>
            </button>
          )}
        </div>
      </header>

      <Modal isOpen={showHelp && Boolean(helpText)} onClose={() => setShowHelp(false)} title={title} size="md">
        <div className="p-5">
          <p className="text-sm text-text-dim leading-relaxed whitespace-pre-line">{helpText}</p>
        </div>
      </Modal>
    </>
  );
}
