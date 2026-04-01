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

export function PageHeader({ icon, title, subtitle, actions, isFetching, onRefresh, helpText }: PageHeaderProps) {
  const { t } = useTranslation();
  const [showHelp, setShowHelp] = useState(false);

  return (
    <>
      <header className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div className="p-1.5 rounded-lg bg-brand/10 text-brand shrink-0">{icon}</div>
          <div className="min-w-0">
            <h1 className="text-base font-extrabold tracking-tight">{title}</h1>
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
            >
              <HelpCircle className="h-4 w-4" />
            </button>
          )}
          {onRefresh && (
            <button
              className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 hover:shadow-sm transition-colors duration-200"
              onClick={onRefresh}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isFetching ? "animate-spin" : ""}`} />
              <span className="hidden sm:inline">{t("common.refresh")}</span>
            </button>
          )}
        </div>
      </header>

      {/* Help Modal */}
      {showHelp && helpText && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
          onClick={() => setShowHelp(false)}
        >
          <div
            className="bg-surface border border-border-subtle rounded-2xl w-full max-w-md shadow-2xl animate-fade-in-scale"
            onClick={e => e.stopPropagation()}
          >
            <div className="flex items-center justify-between p-4 border-b border-border-subtle">
              <div className="flex items-center gap-2">
                <HelpCircle className="h-4 w-4 text-brand" />
                <span className="font-bold text-sm">{title}</span>
              </div>
              <button
                onClick={() => setShowHelp(false)}
                className="p-1.5 hover:bg-main/30 rounded-lg transition-colors text-text-dim hover:text-text-main"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="p-5">
              <p className="text-sm text-text-dim leading-relaxed whitespace-pre-line">{helpText}</p>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
