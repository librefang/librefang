import { useTranslation } from "react-i18next";
import { Activity, AlertTriangle, CheckCircle, Database, Moon, Settings, Sparkles } from "lucide-react";
import { Card } from "../../../components/ui/Card";
import { Badge } from "../../../components/ui/Badge";
import { Button } from "../../../components/ui/Button";
import { useMemoryConfig, useMemoryHealth } from "../../../lib/queries/memory";
import { useAutoDreamStatus } from "../../../lib/queries/autoDream";
import { toastErr } from "../../../lib/errors";
import { formatRelativeMs } from "../formatters";

interface Props {
  onOpenConfig: () => void;
}

export function HealthTab({ onOpenConfig }: Props) {
  const { t, i18n } = useTranslation();
  const configQuery = useMemoryConfig();
  const healthQuery = useMemoryHealth();
  const dreamQuery = useAutoDreamStatus();

  // Each card has three states: the answer is known, the request is still in flight, or it failed.
  // Missing data must never be rendered as a negative fact ("Not reachable", "Disabled"), because an operator reads those as diagnoses.
  const config = configQuery.data;
  const embeddingAvailable = healthQuery.data;
  const dream = dreamQuery.data;
  const tLoading = () => t("common.loading", { defaultValue: "Loading..." });
  const tUnknown = () => t("common.unknown", { defaultValue: "Unknown" });
  const now = Date.now();
  const tNever = () => t("common.never", { defaultValue: "never" });
  const tJustNow = () => t("common.just_now", { defaultValue: "just now" });

  // Roll up the per-agent dream timestamps into a single "system" view:
  //   - lastConsolidatedAt: most recent successful consolidation across all
  //     opted-in agents (zero if nothing has ever fired).
  //   - dreamFailures: count of CURRENTLY ENROLLED agents whose most recent
  //     run terminated in `failed` or `aborted`. Both are treated as broken
  //     from an operator's POV — `aborted` only happens because something
  //     wrong (or a user mashing Stop) triggered the abort path. Scoped to
  //     opted-in so a since-disabled agent doesn't haunt the rollup with a
  //     stale `failed` snapshot forever.
  const optedIn = dream?.agents.filter((a) => a.auto_dream_enabled) ?? [];
  const lastConsolidatedAt = optedIn.reduce(
    (max, a) => Math.max(max, a.last_consolidated_at_ms ?? 0),
    0,
  );
  const dreamFailures = optedIn.filter(
    (a) => a.progress?.status === "failed" || a.progress?.status === "aborted",
  ).length;

  const embeddingTone =
    embeddingAvailable === undefined ? "default" : embeddingAvailable ? "success" : "warning";
  const proactiveOn = config?.proactive_memory?.enabled ?? false;
  const configStatus = config === undefined ? (configQuery.isError ? tUnknown() : tLoading()) : undefined;

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
      {/* Embedding */}
      <Card padding="md">
        <SectionHeader
          icon={<Sparkles className="w-4 h-4 text-brand" />}
          title={t("memory.health_embedding_title", { defaultValue: "Embedding backbone" })}
          tone={embeddingTone}
          status={
            embeddingAvailable === undefined
              ? healthQuery.isError
                ? tUnknown()
                : tLoading()
              : embeddingAvailable
                ? t("memory.health_embedding_ok", { defaultValue: "Reachable" })
                : t("memory.health_embedding_down", { defaultValue: "Not reachable" })
          }
        />
        <DefRow
          label={t("memory.health_provider", { defaultValue: "Provider" })}
          value={
            config === undefined
              ? "—"
              : config.embedding_provider || t("memory.auto", { defaultValue: "auto-detect" })
          }
        />
        <DefRow
          label={t("memory.health_model", { defaultValue: "Model" })}
          value={config?.embedding_model || "—"}
        />
        <DefRow
          label={t("memory.health_api_key_env", { defaultValue: "API key env" })}
          value={
            config === undefined
              ? "—"
              : config.embedding_api_key_env || t("memory.health_unset", { defaultValue: "unset" })
          }
        />
        {embeddingAvailable === undefined ? (
          healthQuery.isError && (
            <LoadFailed
              message={toastErr(healthQuery.error, t("common.error", { defaultValue: "Error" }))}
              retryLabel={t("common.retry", { defaultValue: "Retry" })}
              onRetry={() => void healthQuery.refetch()}
            />
          )
        ) : (
          <p className="text-[11px] text-text-dim mt-3">
            {embeddingAvailable
              ? t("memory.health_embed_hint_ok", {
                  defaultValue:
                    "Liveness probe succeeded — the configured embedding backend responded to a test call.",
                })
              : t("memory.health_embed_hint_fail", {
                  defaultValue:
                    "Liveness probe failed. Memory writes will skip vector indexing until the provider is reachable. Check API key, base URL, and that the local server (Ollama / vLLM / LM Studio) is running.",
                })}
          </p>
        )}
        {configQuery.isError && config === undefined && (
          <LoadFailed
            message={toastErr(configQuery.error, t("common.error", { defaultValue: "Error" }))}
            retryLabel={t("common.retry", { defaultValue: "Retry" })}
            onRetry={() => void configQuery.refetch()}
          />
        )}
      </Card>

      {/* Proactive memory */}
      <Card padding="md">
        <SectionHeader
          icon={<Database className="w-4 h-4 text-brand" />}
          title={t("memory.health_proactive_title", { defaultValue: "Proactive memory" })}
          tone={proactiveOn ? "success" : "default"}
          status={
            configStatus ??
            (proactiveOn
              ? t("memory.health_proactive_on", { defaultValue: "Enabled" })
              : t("memory.health_proactive_off", { defaultValue: "Disabled" }))
          }
        />
        <DefRow
          label={t("memory.health_extraction_model", { defaultValue: "Extraction model" })}
          value={
            config === undefined
              ? "—"
              : config.proactive_memory?.extraction_model || t("memory.health_use_default", { defaultValue: "kernel default" })
          }
        />
        <DefRow
          label={t("memory.health_auto_memorize", { defaultValue: "Auto memorize" })}
          value={
            config === undefined
              ? "—"
              : (config.proactive_memory?.auto_memorize ?? true)
                ? t("common.on", { defaultValue: "ON" })
                : t("common.off", { defaultValue: "Off" })
          }
        />
        <DefRow
          label={t("memory.health_auto_retrieve", { defaultValue: "Auto retrieve" })}
          value={
            config === undefined
              ? "—"
              : (config.proactive_memory?.auto_retrieve ?? true)
                ? t("common.on", { defaultValue: "ON" })
                : t("common.off", { defaultValue: "Off" })
          }
        />
        <DefRow
          label={t("memory.health_decay_rate", { defaultValue: "Decay rate" })}
          value={config?.decay_rate != null ? config.decay_rate.toFixed(3) : "—"}
        />
      </Card>

      {/* Auto-Dream */}
      <Card padding="md">
        <SectionHeader
          icon={<Moon className="w-4 h-4 text-purple-400" />}
          title={t("memory.health_dream_title", { defaultValue: "Auto-Dream" })}
          tone={dream?.enabled ? (dreamFailures > 0 ? "warning" : "success") : "default"}
          status={
            dream === undefined
              ? dreamQuery.isError
                ? tUnknown()
                : tLoading()
              : dream.enabled
                ? dreamFailures > 0
                  ? t("memory.health_dream_with_errors", {
                      defaultValue: "Running with errors",
                    })
                  : t("memory.health_dream_on", { defaultValue: "Running" })
                : t("memory.health_dream_off", { defaultValue: "Disabled" })
          }
        />
        <DefRow
          label={t("memory.health_dream_enrolled", { defaultValue: "Agents enrolled" })}
          value={dream === undefined ? "—" : `${optedIn.length} / ${dream.agents.length}`}
        />
        <DefRow
          label={t("memory.health_dream_last", {
            defaultValue: "Last successful consolidation",
          })}
          value={
            dream === undefined
              ? "—"
              : formatRelativeMs(lastConsolidatedAt, now, i18n.language, tNever, tJustNow)
          }
        />
        <DefRow
          label={t("memory.health_dream_failed", {
            defaultValue: "Enrolled agents with broken last run",
          })}
          value={dream === undefined ? "—" : String(dreamFailures)}
          tone={dreamFailures > 0 ? "warning" : undefined}
        />
        {dreamQuery.isError && dream === undefined && (
          <LoadFailed
            message={toastErr(dreamQuery.error, t("common.error", { defaultValue: "Error" }))}
            retryLabel={t("common.retry", { defaultValue: "Retry" })}
            onRetry={() => void dreamQuery.refetch()}
          />
        )}
        {dreamFailures > 0 && (
          <p className="text-[11px] text-warning/90 mt-3 flex items-start gap-1.5">
            <AlertTriangle className="w-3 h-3 mt-0.5 shrink-0" />
            {t("memory.health_dream_warn", {
              defaultValue:
                "One or more enrolled agents' last dream ended in failure or abort. Open the Auto-Dream tab and check the per-agent error message.",
            })}
          </p>
        )}
      </Card>

      {/* Actions / pointer back to config drawer */}
      <Card padding="md">
        <SectionHeader
          icon={<Activity className="w-4 h-4 text-brand" />}
          title={t("memory.health_actions_title", { defaultValue: "Tune behaviour" })}
        />
        <p className="text-[11px] text-text-dim mb-3">
          {t("memory.health_actions_desc", {
            defaultValue:
              "All knobs above live in config.toml. The Settings drawer mirrors them with validation and dropdowns.",
          })}
        </p>
        <Button variant="secondary" size="sm" onClick={onOpenConfig}>
          <Settings className="w-3.5 h-3.5 mr-1.5" />
          {t("memory.health_open_config", { defaultValue: "Open configuration" })}
        </Button>
      </Card>
    </div>
  );
}

function SectionHeader({
  icon,
  title,
  tone,
  status,
}: {
  icon: React.ReactNode;
  title: string;
  tone?: "success" | "warning" | "default";
  status?: string;
}) {
  return (
    <div className="flex items-center justify-between mb-3">
      <div className="flex items-center gap-2">
        {icon}
        <h4 className="text-sm font-bold">{title}</h4>
      </div>
      {status && (
        <Badge variant={tone === "success" ? "success" : tone === "warning" ? "warning" : "default"}>
          {tone === "success" && <CheckCircle className="w-3 h-3 mr-1 inline" />}
          {tone === "warning" && <AlertTriangle className="w-3 h-3 mr-1 inline" />}
          {status}
        </Badge>
      )}
    </div>
  );
}

function LoadFailed({
  message,
  retryLabel,
  onRetry,
}: {
  message: string;
  retryLabel: string;
  onRetry: () => void;
}) {
  return (
    <div role="alert" className="mt-3 flex items-start justify-between gap-2 text-[11px] text-error">
      <span className="flex items-start gap-1.5">
        <AlertTriangle className="w-3 h-3 mt-0.5 shrink-0" />
        {message}
      </span>
      <Button variant="secondary" size="sm" onClick={onRetry}>
        {retryLabel}
      </Button>
    </div>
  );
}

function DefRow({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "warning";
}) {
  return (
    <div className="flex items-baseline justify-between py-1 border-t border-border-subtle/30 first:border-t-0">
      <span className="text-[11px] text-text-dim">{label}</span>
      <span
        className={`text-xs font-mono ${tone === "warning" ? "text-warning" : "text-text-main"}`}
      >
        {value}
      </span>
    </div>
  );
}
