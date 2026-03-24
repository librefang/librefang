import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, HelpCircle, X } from "lucide-react";

interface PageHeaderProps {
  icon: ReactNode;
  badge: string;
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  isFetching?: boolean;
  onRefresh?: () => void;
  helpText?: string;
}

export function PageHeader({ icon, badge, title, subtitle, actions, isFetching, onRefresh, helpText }: PageHeaderProps) {
  const { t } = useTranslation();
  const [showHelp, setShowHelp] = useState(false);

  return (
    <>
      <header className="flex flex-col justify-between gap-2 sm:gap-4 sm:flex-row sm:items-end">
        <div className="min-w-0">
          <div className="hidden sm:flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <div className="p-1 rounded-md bg-brand/10">{icon}</div>
            {badge}
          </div>
          <h1 className="text-xl sm:text-3xl font-extrabold tracking-tight md:text-4xl bg-gradient-to-r from-text-main to-text-dim/70 bg-clip-text sm:mt-3">{title}</h1>
          {subtitle && <p className="mt-0.5 sm:mt-1.5 text-text-dim font-medium max-w-2xl text-[11px] sm:text-sm hidden sm:block">{subtitle}</p>}
        </div>
        <div className="flex items-center gap-2 sm:gap-3 shrink-0 flex-wrap">
          {actions}
          {helpText && (
            <button
              onClick={() => setShowHelp(!showHelp)}
              className="flex h-8 sm:h-9 w-8 sm:w-9 items-center justify-center rounded-xl border border-border-subtle bg-surface text-text-dim hover:text-brand hover:border-brand/30 transition-colors duration-200"
              title={t("common.help", { defaultValue: "Help" })}
            >
              <HelpCircle className="h-4 w-4" />
            </button>
          )}
          {onRefresh && (
            <button
              className="flex h-8 sm:h-9 items-center gap-1.5 sm:gap-2 rounded-xl border border-border-subtle bg-surface px-3 sm:px-4 text-xs sm:text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 hover:shadow-sm transition-colors duration-200"
              onClick={onRefresh}
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${isFetching ? "animate-spin" : ""}`}
              />
              <span className="hidden sm:inline">{t("common.refresh")}</span>
            </button>
          )}
        </div>
      </header>
      {showHelp && helpText && (
        <div className="rounded-xl border border-brand/20 bg-brand/5 px-4 py-3 text-sm text-text-dim leading-relaxed animate-fade-in-up">
          <div className="flex items-start justify-between gap-3">
            <p className="whitespace-pre-line">{helpText}</p>
            <button onClick={() => setShowHelp(false)} className="shrink-0 text-text-dim/50 hover:text-text-dim">
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}
    </>
  );
}
