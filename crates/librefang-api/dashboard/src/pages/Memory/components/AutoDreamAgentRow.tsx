import { useTranslation } from "react-i18next";
import { Moon, Play, Square, XCircle } from "lucide-react";
import { Badge } from "../../../components/ui/Badge";
import { Button } from "../../../components/ui/Button";
import type { AutoDreamAgentStatus } from "../../../api";
import { formatRelativeMs, formatHours } from "../formatters";

interface Props {
  agent: AutoDreamAgentStatus;
  // Disable the manual "Dream now" affordance when global auto-dream is off
  // (the run would no-op on the backend). Per-agent enroll toggle and Abort
  // stay live because an already-running dream still needs an out.
  disabled: boolean;
  // Hide the redundant agent name header when this row is embedded in a
  // larger agent-scoped card that already shows the name. Default: show.
  hideAgentName?: boolean;
  onTrigger: (id: string) => void;
  onAbort: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  triggerPending: boolean;
  abortPending: boolean;
  togglePending: boolean;
}

export function AutoDreamAgentRow({
  agent,
  disabled,
  hideAgentName,
  onTrigger,
  onAbort,
  onToggle,
  triggerPending,
  abortPending,
  togglePending,
}: Props) {
  const { t, i18n } = useTranslation();
  const now = Date.now();
  const progress = agent.progress;
  const running = progress?.status === "running";
  const lastTurn = progress?.turns[progress.turns.length - 1];
  const optedIn = agent.auto_dream_enabled;
  const tNever = () => t("common.never", { defaultValue: "never" });
  const durationUnits = {
    minute: t("settings.auto_dream_dur_minute"),
    hour: t("settings.auto_dream_dur_hour"),
    day: t("settings.auto_dream_dur_day"),
    week: t("settings.auto_dream_dur_week"),
  };

  return (
    <div className="rounded-lg border border-border-subtle/50 bg-main">
      <div className="flex items-center justify-between gap-3 px-3 py-2">
        <div className="flex items-start gap-2 min-w-0 flex-1">
          <Moon
            className={`w-4 h-4 shrink-0 mt-0.5 ${
              optedIn
                ? running
                  ? "text-purple-400 animate-pulse"
                  : "text-purple-400"
                : "text-text-dim"
            }`}
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 flex-wrap">
              {!hideAgentName && (
                <p className="text-sm font-medium truncate">{agent.agent_name}</p>
              )}
              {progress && (
                <Badge
                  variant={
                    progress.status === "running"
                      ? "info"
                      : progress.status === "completed"
                      ? "success"
                      : progress.status === "aborted"
                      ? "warning"
                      : "error"
                  }
                >
                  {t(`settings.auto_dream_status_${progress.status}`, progress.status)}
                </Badge>
              )}
            </div>
            {optedIn ? (
              <p className="text-[11px] text-text-dim mt-0.5">
                {t("settings.auto_dream_last", "Last")}:{" "}
                {formatRelativeMs(agent.last_consolidated_at_ms, now, i18n.language, tNever)}
                {" · "}
                {t("settings.auto_dream_next", "Next")}:{" "}
                {formatRelativeMs(agent.next_eligible_at_ms, now, i18n.language, tNever)}
                {" · "}
                {agent.effective_min_sessions > 0 ? (
                  <span
                    title={t(
                      "settings.auto_dream_sessions_progress_title",
                      "Sessions touched since last dream / required threshold",
                    )}
                  >
                    {agent.sessions_since_last}/{agent.effective_min_sessions}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </span>
                ) : (
                  <>
                    {agent.sessions_since_last}{" "}
                    {t("settings.auto_dream_sessions_since", "sessions since")}
                  </>
                )}
                {" · "}
                <span
                  title={t(
                    "settings.auto_dream_effective_title",
                    "Resolved threshold — manifest override or global default",
                  )}
                >
                  {t("settings.auto_dream_every", "every")}{" "}
                  {formatHours(agent.effective_min_hours, durationUnits)}
                </span>
              </p>
            ) : running ? (
              // Agent was toggled off while a manual dream was already in
              // flight. Keep the operator informed — the run continues to
              // completion or abort, and the abort button stays live.
              <p className="text-[11px] text-text-dim italic mt-0.5">
                {t(
                  "settings.auto_dream_opt_out_running",
                  "Disabled mid-dream — the current run will finish or can be aborted.",
                )}
              </p>
            ) : (
              <p className="text-[11px] text-text-dim italic mt-0.5">
                {t(
                  "settings.auto_dream_opt_in_hint",
                  "Not enrolled — toggle on to include in the scheduler.",
                )}
              </p>
            )}
          </div>
        </div>
        <div className="flex gap-2 shrink-0 items-center">
          <label
            className="flex items-center gap-1.5 cursor-pointer select-none"
            title={t("settings.auto_dream_toggle_title", "Opt this agent in or out")}
          >
            <input
              type="checkbox"
              checked={optedIn}
              disabled={togglePending}
              onChange={(e) => onToggle(agent.agent_id, e.target.checked)}
              className="w-3.5 h-3.5 accent-purple-500"
            />
            <span className="text-[11px] text-text-dim">
              {optedIn
                ? t("settings.auto_dream_enrolled", "Enrolled")
                : t("settings.auto_dream_not_enrolled", "Off")}
            </span>
          </label>
          {running && agent.can_abort && (
            // Surface the abort affordance even when the agent has been toggled
            // off mid-dream — otherwise the in-flight operation keeps spending
            // tokens with no UI to stop it.
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onAbort(agent.agent_id)}
              disabled={abortPending}
            >
              <Square className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_abort", "Abort")}
            </Button>
          )}
          {optedIn && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onTrigger(agent.agent_id)}
              disabled={triggerPending || disabled || running}
              title={disabled ? t("settings.auto_dream_off", "Disabled") : undefined}
            >
              <Play className="w-3.5 h-3.5 mr-1.5" />
              {t("settings.auto_dream_trigger", "Dream now")}
            </Button>
          )}
        </div>
      </div>

      {progress && (progress.status !== "completed" || progress.memories_touched.length > 0) && (
        <div className="px-3 pb-2 pt-1 border-t border-border-subtle/30 space-y-1">
          <p className="text-[10px] text-text-dim">
            <span className="uppercase tracking-wider">
              {t("settings.auto_dream_phase", "Phase")}:
            </span>{" "}
            <span className="font-mono">{progress.phase}</span>
            {" · "}
            {progress.tool_use_count}{" "}
            {t("settings.auto_dream_tool_calls", "tool calls")}
            {progress.memories_touched.length > 0 && (
              <>
                {" · "}
                {progress.memories_touched.length}{" "}
                {t("settings.auto_dream_memories_touched", "memories touched")}
              </>
            )}
          </p>
          {lastTurn && lastTurn.text && (
            <p className="text-[11px] text-text-muted line-clamp-2 italic">
              &ldquo;{lastTurn.text}&rdquo;
            </p>
          )}
          {progress.error && (
            <p className="text-[11px] text-red-500">
              <XCircle className="w-3 h-3 inline mr-1" />
              {progress.error}
            </p>
          )}
          {/* Cache-hit visibility. Since the forkedAgent migration, dreams fork
              off the parent turn and hit Anthropic's prompt cache on the
              (system + tools + messages) prefix. Surfacing the hit rate here
              lets operators see the actual cost win. Only shown for completed
              dreams (usage is populated then) and only when there actually was
              input (avoids 0/0 noise). */}
          {progress.usage && progress.usage.input_tokens > 0 && (
            <p className="text-[10px] text-text-dim">
              <span className="uppercase tracking-wider">
                {t("settings.auto_dream_cache", "Cache")}:
              </span>{" "}
              {(() => {
                const u = progress.usage!;
                const totalIn =
                  u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens;
                const hitPct =
                  totalIn > 0 ? Math.round((u.cache_read_input_tokens / totalIn) * 100) : 0;
                return (
                  <span
                    title={t(
                      "settings.auto_dream_cache_title",
                      "Prompt cache hit rate for this dream — higher means more of the prefix came from Anthropic's cache instead of being re-billed.",
                    )}
                  >
                    <span className="font-mono">{hitPct}%</span>{" "}
                    ({u.cache_read_input_tokens.toLocaleString()}/{totalIn.toLocaleString()} tok)
                  </span>
                );
              })()}
              {typeof progress.usage.cost_usd === "number" && (
                <>
                  {" · "}
                  <span
                    title={t(
                      "settings.auto_dream_cost_title",
                      "Measured provider cost for this dream turn (input + output, cached tokens billed at the reduced rate).",
                    )}
                  >
                    ${progress.usage.cost_usd.toFixed(5)}
                  </span>
                </>
              )}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
