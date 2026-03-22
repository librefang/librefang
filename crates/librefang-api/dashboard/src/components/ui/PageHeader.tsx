import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";

interface PageHeaderProps {
  icon: ReactNode;
  badge: string;
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  isFetching?: boolean;
  onRefresh?: () => void;
}

export function PageHeader({ icon, badge, title, subtitle, actions, isFetching, onRefresh }: PageHeaderProps) {
  const { t } = useTranslation();

  return (
    <header className="flex flex-col justify-between gap-3 sm:gap-4 sm:flex-row sm:items-end">
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <div className="p-1 rounded-md bg-brand/10">{icon}</div>
          {badge}
        </div>
        <h1 className="mt-2 sm:mt-3 text-2xl sm:text-3xl font-extrabold tracking-tight md:text-4xl bg-gradient-to-r from-text-main to-text-dim/70 bg-clip-text">{title}</h1>
        {subtitle && <p className="mt-1 sm:mt-1.5 text-text-dim font-medium max-w-2xl text-xs sm:text-sm">{subtitle}</p>}
      </div>
      <div className="flex items-center gap-2 sm:gap-3 shrink-0 flex-wrap">
        {actions}
        {onRefresh && (
          <button
            className="flex h-8 sm:h-9 items-center gap-1.5 sm:gap-2 rounded-xl border border-border-subtle bg-surface px-3 sm:px-4 text-xs sm:text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 hover:shadow-sm transition-all duration-200"
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
  );
}
