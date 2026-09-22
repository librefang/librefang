import { useTranslation } from "react-i18next";
import { ChevronDown, Cpu, MessageCircle } from "lucide-react";
import type { AgentEventRow, AgentStats24h } from "../api";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { MarkdownContent } from "./ui/MarkdownContent";
import { formatRelativeTime } from "../lib/datetime";
import { formatNumber } from "../lib/format";
import { getStatusVariant } from "../lib/status";

/**
 * The agent at a glance: what it is, what it has been doing, what it is costing,
 * and the tail of its live conversation.
 *
 * This is the whole of the "logs & info" tab above the sub-tab bar. It used to
 * be spread across the panel header, a Configure drawer and a "conversation"
 * tab, which is how the token footprint ended up at the bottom of a drawer that
 * had to be opened to read a number nobody was editing.
 *
 * Presentational on purpose, and props-only: `AgentsPage` mounts some twenty
 * hooks and has no render harness, so a brief that read its own queries would
 * be unreachable from a test exactly like the code it replaced. Everything it
 * shows is data its caller already has.
 */
export interface AgentBriefAgent {
  id: string;
  name: string;
  /** Runtime state ("running", "suspended", "crashed", …). */
  state?: string;
  model?: { provider?: string; model?: string } | null;
  model_name?: string;
  profile?: string;
  last_active?: string;
  /** Tokens the agent's identity and tool definitions inject into every
   *  request. `null`/absent means the daemon did not report one — which is not
   *  the same as a reported zero (see `hasTokenFootprintData`). */
  injected_footprint_tokens?: number | null;
}

export interface AgentBriefMessage {
  role?: string;
  content?: unknown;
}

export interface AgentBriefProps {
  agent: AgentBriefAgent;
  stats?: AgentStats24h | null;
  /** Recent turns, for the "recent calls" half of the footprint panel. */
  events?: AgentEventRow[];
  /** Messages of the agent's latest session, already fetched by the caller. */
  messages?: AgentBriefMessage[];
  conversationLoading?: boolean;
  /** False when the agent has no session yet: distinguishes "nothing sent"
   *  from "still loading". */
  hasSession?: boolean;
  /** Declared tool count, and the one-line summary under it (skill names, or
   *  "N configured"). Computed by the caller from the detail payload. */
  toolsCount?: number | null;
  toolsMeta?: string;
  onOpenChat?: () => void;
}

/** The text of a message whose `content` is a string or a block array. */
export function messageText(m: AgentBriefMessage): string {
  if (typeof m.content === "string") return m.content;
  if (Array.isArray(m.content)) {
    return (m.content as Array<{ type?: string; text?: string }>)
      .filter((b) => b.type === "text" || b.text)
      .map((b) => b.text ?? "")
      .join(" ");
  }
  return "";
}

/** Percentage change of `cur` against `p`, as the KPI subtext renders it. */
function pctDelta(cur: number, p: number): string {
  if (p === 0) return cur > 0 ? "new" : "—";
  const d = ((cur - p) / p) * 100;
  const sign = d >= 0 ? "+" : "−";
  return `${sign}${Math.abs(d).toFixed(0)}%`;
}

function usdDelta(cur: number, p: number): string {
  const d = cur - p;
  if (Math.abs(d) < 0.01) return cur === 0 && p === 0 ? "—" : "≈$0.00";
  const sign = d >= 0 ? "+" : "−";
  return `${sign}$${Math.abs(d).toFixed(2)}`;
}

function msDelta(cur: number, p: number): string {
  if (cur === 0 && p === 0) return "—";
  if (p === 0) return "new";
  const d = cur - p;
  const sign = d >= 0 ? "+" : "−";
  return Math.abs(d) >= 1000
    ? `${sign}${(Math.abs(d) / 1000).toFixed(1)}s`
    : `${sign}${Math.abs(Math.round(d))}ms`;
}

export function AgentBrief({
  agent,
  stats,
  events = [],
  messages = [],
  conversationLoading = false,
  hasSession = true,
  toolsCount = null,
  toolsMeta = "—",
  onOpenChat,
}: AgentBriefProps) {
  const { t } = useTranslation();

  const sessions24h = stats?.sessions_24h ?? 0;
  const cost24h = stats?.cost_24h ?? 0;
  const p95Ms = stats?.p95_latency_ms ?? 0;
  const samples = stats?.samples ?? 0;
  const activeNow = stats?.active_now ?? 0;
  const prev = stats?.prev;

  const visibleMessages = messages
    .filter((m) => m.role === "user" || m.role === "assistant")
    .slice(-5);

  const kpiTiles = [
    {
      l: t("agents.kpi.sessions", { defaultValue: "Sessions · 24h" }),
      v: String(sessions24h),
      m: prev
        ? activeNow > 0
          ? `${activeNow} live · ${pctDelta(sessions24h, prev.sessions_24h)}`
          : pctDelta(sessions24h, prev.sessions_24h)
        : activeNow > 0
          ? `${activeNow} live`
          : "—",
    },
    {
      l: t("agents.kpi.cost", { defaultValue: "Cost · 24h" }),
      v: `$${cost24h.toFixed(2)}`,
      m: prev ? usdDelta(cost24h, prev.cost_24h) : "—",
    },
    {
      l: t("agents.kpi.p95", { defaultValue: "P95 latency" }),
      v:
        p95Ms > 0
          ? p95Ms >= 1000
            ? `${(p95Ms / 1000).toFixed(2)}s`
            : `${Math.round(p95Ms)}ms`
          : "—",
      m:
        prev && (samples > 0 || prev.p95_latency_ms > 0)
          ? msDelta(p95Ms, prev.p95_latency_ms)
          : samples > 0
            ? t("agents.kpi.samples", { count: samples, defaultValue: "{{count}} samples" })
            : "—",
    },
    {
      l: t("agents.kpi.tools", { defaultValue: "Tools" }),
      v: String(toolsCount),
      m: toolsMeta,
    },
  ];

  return (
    <div className="flex flex-col gap-3">
      {/* Identity line: what it is and when it was last doing anything. */}
      <div className="flex flex-wrap items-center gap-2 text-[11.5px] text-text-dim">
        <Badge variant={getStatusVariant(agent.state)} dot className="shrink-0">
          {agent.state
            ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state })
            : t("common.idle")}
        </Badge>
        <span className="inline-flex items-center gap-1 min-w-0">
          <Cpu className="w-3 h-3 shrink-0" />
          <span className="font-mono truncate">
            {agent.model?.model || agent.model_name || t("agents.brief.no_model", { defaultValue: "no model" })}
          </span>
        </span>
        <span className="truncate">
          {t("agents.brief.last_activity", { defaultValue: "Last activity" })}:{" "}
          {agent.last_active
            ? formatRelativeTime(agent.last_active)
            : t("agents.brief.never", { defaultValue: "never" })}
        </span>
      </div>

      {/* KPI tiles — Sessions · Cost · P95 · Tools (matches design canvas).
          Backed by GET /api/agents/{id}/stats so values are accurate even
          when the agent hasn't appeared in the global session list page. */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
        {kpiTiles.map((s) => (
          <div key={s.l} className="px-3 py-2 rounded-md bg-main/60 border border-border-subtle min-w-0">
            <div className="text-[10px] uppercase font-semibold text-text-dim tracking-[0.08em] truncate">
              {s.l}
            </div>
            <div className="font-mono font-semibold text-[17px] mt-1 truncate tabular-nums text-text-main">
              {s.v}
            </div>
            <div className="text-[10.5px] text-text-dim/80 mt-0.5 truncate">{s.m}</div>
          </div>
        ))}
      </div>

      {/* Token footprint — informational, so it folds instead of taking a
          corner of a drawer it had to be opened to reach. Native
          `<details>`: keyboard and toggle behaviour come free. */}
      {hasTokenFootprintData(agent.injected_footprint_tokens) && (
        <details className="group rounded-lg border border-border-subtle/60 bg-main/30">
          <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2 select-none">
            <span className="text-[11px] font-bold text-text-dim group-open:text-text-main">
              {t("agents.token_usage_title", { defaultValue: "Token footprint" })}
            </span>
            <span className="flex items-center gap-2">
              <span className="font-mono text-[11px] text-text-dim tabular-nums">
                {formatNumber(agent.injected_footprint_tokens as number)}
              </span>
              <ChevronDown className="h-3.5 w-3.5 text-text-dim transition-transform group-open:rotate-180" />
            </span>
          </summary>
          <div className="space-y-1.5 px-3 pb-3">
            <div className="flex justify-between text-[11px] border-t border-border/40 pt-1.5">
              <span className="font-bold">
                {t("agents.token_injected_total", { defaultValue: "Injected per request" })}
              </span>
              <span className="font-mono font-bold">
                {formatNumber(agent.injected_footprint_tokens as number)}
              </span>
            </div>
            {events.length > 0 && (
              <div className="border-t border-border/40 pt-1.5 space-y-1">
                <p className="text-[10px] text-text-dim">
                  {t("agents.token_recent", { defaultValue: "Recent calls" })}
                </p>
                {events.slice(0, 5).map((call, i) => (
                  <div key={`${call.timestamp}-${i}`} className="flex justify-between text-[10px]">
                    <span className="text-text-dim truncate">{call.model}</span>
                    <span className="font-mono">
                      {call.input_tokens}/{call.output_tokens} · ${call.cost_usd.toFixed(4)}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </details>
      )}

      {/* Live conversation — the tail of the newest session. A preview only:
          the full thread lives behind Open chat. */}
      <div className="flex flex-col gap-2.5">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
          {t("agents.detail.live_conversation", { defaultValue: "Live conversation" })}
        </div>
        {conversationLoading && hasSession ? (
          <div className="text-[12px] text-text-dim italic">
            {t("common.loading", { defaultValue: "Loading..." })}
          </div>
        ) : visibleMessages.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_conversation", {
              defaultValue: "No conversation yet — open the chat to send the first message.",
            })}
          </div>
        ) : (
          visibleMessages.map((m, i) => {
            const isUser = m.role === "user";
            const txt = messageText(m).trim();
            if (!txt) return null;
            // Truncate first, then render. The full conversation lives behind
            // the "Open chat" button — this is a preview only.
            const preview = txt.length > 280 ? `${txt.slice(0, 280)}…` : txt;
            return (
              <div key={i} className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                <div
                  className={`max-w-[78%] rounded-lg px-3 py-2 text-[12.5px] break-words border ${
                    isUser
                      ? "bg-brand/10 border-brand/30 text-text-main"
                      : "bg-main/60 border-border-subtle text-text-main"
                  }`}
                >
                  {isUser ? (
                    <span className="whitespace-pre-wrap">{preview}</span>
                  ) : (
                    <MarkdownContent>{preview}</MarkdownContent>
                  )}
                </div>
              </div>
            );
          })
        )}
        {onOpenChat && (
          <div className="flex justify-start">
            <Button
              variant="primary"
              size="sm"
              leftIcon={<MessageCircle className="h-3.5 w-3.5" />}
              onClick={onOpenChat}
            >
              {t("agents.detail.open_chat", { defaultValue: "Open chat" })}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Whether a token footprint was reported at all.
 *
 * A reported `0` is data — an agent with tools disabled and no system prompt
 * really does inject nothing — while a missing field means the daemon did not
 * answer, and rendering a confident `0` for it would invent a measurement.
 */
export function hasTokenFootprintData(
  tokens: number | null | undefined,
): tokens is number {
  return tokens !== null && tokens !== undefined;
}
