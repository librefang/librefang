import { useTranslation } from "react-i18next";
import type { UseQueryResult } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import type { AgentKvPair } from "../../../api";
import { formatDateTime } from "../../../lib/datetime";
import { formatKvValue } from "../formatters";
import { KV_VALUE_TRUNCATE, KV_TITLE_TRUNCATE } from "../constants";

interface Props {
  kvQuery: UseQueryResult<AgentKvPair[]>;
}

// Receives the per-agent KV query result from the surrounding tab — a single
// `useQueries` observer batches every agent's lookup so this row component
// stays presentational (no per-row hook subscription, no N+1 churn).
export function AgentKvRows({ kvQuery }: Props) {
  const { t } = useTranslation();

  if (kvQuery.isLoading) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-text-dim">
          <Loader2 className="w-3.5 h-3.5 animate-spin inline" />
        </td>
      </tr>
    );
  }
  if (kvQuery.isError) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-error">
          {kvQuery.error instanceof Error ? kvQuery.error.message : t("common.error")}
        </td>
      </tr>
    );
  }

  const pairs = kvQuery.data ?? [];
  if (pairs.length === 0) {
    return (
      <tr>
        <td colSpan={4} className="px-3 py-2 text-xs text-text-dim/60 italic">
          {t("memory.kv_empty", { defaultValue: "No KV entries" })}
        </td>
      </tr>
    );
  }

  return (
    <>
      {pairs.map((pair: AgentKvPair) => {
        const formatted = formatKvValue(pair.value);
        const truncated =
          formatted.length > KV_VALUE_TRUNCATE
            ? formatted.slice(0, KV_VALUE_TRUNCATE) + "…"
            : formatted;
        const titlePreview =
          formatted.length > KV_TITLE_TRUNCATE
            ? formatted.slice(0, KV_TITLE_TRUNCATE) + "…"
            : formatted;
        return (
          <tr key={pair.key} className="border-t border-border-subtle/40">
            <td className="px-3 py-2 text-xs font-mono break-all">{pair.key}</td>
            <td
              className="px-3 py-2 text-xs font-mono text-text-dim break-all"
              title={titlePreview}
            >
              {truncated}
            </td>
            <td className="px-3 py-2 text-xs text-text-dim">{pair.source ?? "-"}</td>
            <td className="px-3 py-2 text-xs text-text-dim">
              {pair.created_at ? formatDateTime(pair.created_at) : "-"}
            </td>
          </tr>
        );
      })}
    </>
  );
}
