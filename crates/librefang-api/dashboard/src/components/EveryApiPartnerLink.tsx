import { ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";

import { EVERYAPI_PARTNER } from "../lib/partner";

export function EveryApiPartnerLink({ collapsed }: { collapsed: boolean }) {
  const { t } = useTranslation();

  return (
    <a
      href={EVERYAPI_PARTNER.pageUrl}
      target="_blank"
      rel="noopener noreferrer"
      title={collapsed ? t("partner.everyapi_label") : undefined}
      aria-label={collapsed ? t("partner.everyapi_label") : undefined}
      className={`mx-2 mb-2 flex min-h-10 items-center rounded-lg border border-brand/20 bg-brand/5 text-text-dim transition-colors hover:border-brand/40 hover:bg-brand/10 hover:text-brand ${
        collapsed ? "justify-center px-0" : "gap-2.5 px-3"
      }`}
    >
      <span className="flex h-6 w-6 shrink-0 items-center justify-center overflow-hidden rounded-md border border-border bg-surface">
        <img src={EVERYAPI_PARTNER.logoUrl} alt="" className="h-full w-full object-contain" aria-hidden="true" />
      </span>
      {!collapsed && (
        <span className="min-w-0 flex-1">
          <span className="block text-[10px] font-semibold uppercase tracking-[0.1em] text-brand">
            {t("partner.official")}
          </span>
          {" "}
          <span className="block truncate text-xs font-medium text-text">{t("partner.librefang_everyapi")}</span>
        </span>
      )}
      {!collapsed && <ExternalLink className="h-3 w-3 shrink-0" aria-hidden="true" />}
    </a>
  );
}
