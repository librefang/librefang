/**
 * Federated hub source bar for the Skills page. Renders an "All hubs"
 * pill plus one pill per configured hub with a health dot, glyph, and
 * count of items currently visible from that source. Counts are passed
 * in from the page so this stays a dumb presentational component — the
 * page already has the typed query results and knows what's filtered.
 *
 * Companion `HubBadge` is used inside skill cards / detail modals to
 * stamp every entry with its origin.
 */
import { Boxes, Check, Loader2, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { copyToClipboard } from "../lib/clipboard";
import { SKILL_HUBS, type SkillHub, type SkillHubId, getSkillHub } from "../lib/skillHubs";

export type HubFilter = "all" | SkillHubId;

/**
 * A hub's wire health for the source bar dot.
 *
 * `"unknown"` is the state the bar used to be unable to express: a hub whose query is disabled — the page has never asked it anything, so it is neither live nor down.
 * Collapsing that into `"live"` painted a green dot on a marketplace that had not been contacted, which is how a dead hub read as healthy until someone clicked it (#7387).
 */
export type HubHealth = "live" | "checking" | "down" | "unknown";

/** The subset of a React Query result the health mapping reads. */
export interface HubQueryState {
  isError: boolean;
  isFetching: boolean;
  /** False until the query has resolved once — a disabled query never sets it. */
  isFetched: boolean;
}

/**
 * Map a hub's query state onto its wire health.
 *
 * Order matters: an errored query that React Query is already retrying reports both `isError` and `isFetching`, and the honest answer there is still "down".
 */
export function hubHealthFrom(query: HubQueryState): HubHealth {
  if (query.isError) return "down";
  if (query.isFetching) return "checking";
  return query.isFetched ? "live" : "unknown";
}

export type HubCounts = Partial<Record<SkillHubId, number>>;
export type HubHealthMap = Partial<Record<SkillHubId, HubHealth>>;

interface SkillHubBarProps {
  hubFilter: HubFilter;
  onChange: (filter: HubFilter) => void;
  /** Item counts by hub. Missing entries render as "—". */
  counts?: HubCounts;
  /** Per-hub health used to render the colored dot. Missing → "live". */
  health?: HubHealthMap;
  /** Total across all hubs for the "All hubs" pill. */
  totalCount?: number;
}

function dotColor(h: HubHealth | undefined): string {
  if (h === "down") return "var(--color-error, #ef4444)";
  if (h === "checking") return "var(--color-warning, #f59e0b)";
  if (h === "unknown") return "var(--color-text-dim, #6b7280)";
  return "var(--color-success, #22c55e)";
}

/** Translation key naming each health state, used for the dot's screen-reader text and the pill tooltip. */
function healthLabelKey(h: HubHealth | undefined): string {
  if (h === "down") return "skills.hub_unreachable";
  if (h === "checking") return "skills.hub_checking";
  if (h === "unknown") return "skills.hub_not_checked";
  return "skills.hub_live";
}

/**
 * The colored dot, plus the same information as text for anyone not looking at colors.
 * The dot stays `aria-hidden`; the sibling `sr-only` span carries the hub name and its state, so a dead hub is reachable without hovering the pill for a `title`.
 */
function HubHealthDot({ hubName, health }: { hubName: string; health?: HubHealth }) {
  const { t } = useTranslation();
  const color = dotColor(health);
  return (
    <>
      <span
        aria-hidden="true"
        className="inline-block w-1.5 h-1.5 rounded-full shrink-0"
        style={{ background: color, boxShadow: `0 0 0 2px ${color}33` }}
      />
      <span className="sr-only">
        {t("skills.hub_status", { hub: hubName, status: t(healthLabelKey(health)) })}
      </span>
    </>
  );
}

export function HubBadge({
  hub,
  size = "sm",
}: {
  hub: SkillHubId;
  size?: "sm" | "lg";
}) {
  const h = getSkillHub(hub);
  if (!h) return null;
  const px = size === "lg" ? "text-[11px] px-2 py-[3px]" : "text-[10px] px-1.5 py-[2px]";
  return (
    <span
      className={`inline-flex items-center gap-1 rounded font-mono font-semibold ${px}`}
      style={{
        background: `${h.color}14`,
        color: h.color,
        border: `1px solid ${h.color}40`,
      }}
    >
      <span className={size === "lg" ? "text-[11px]" : "text-[10px]"}>{h.glyph}</span>
      {h.name}
    </span>
  );
}

function HubPill({
  hub,
  active,
  count,
  health,
  onClick,
}: {
  hub: SkillHub;
  active: boolean;
  count: number | undefined;
  health: HubHealth | undefined;
  onClick: () => void;
}) {
  const { t } = useTranslation();
  const statusText = t(healthLabelKey(health));
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={`px-2.5 py-1.5 text-[12px] rounded-md border inline-flex items-center gap-1.5 transition-colors cursor-pointer font-medium ${
        active ? "" : "hover:bg-main/40"
      }`}
      style={{
        background: active ? `${hub.color}14` : "transparent",
        borderColor: active ? `${hub.color}60` : "var(--color-border-subtle)",
        color: active ? hub.color : "var(--color-text-main)",
      }}
      title={`${hub.domain} · ${statusText}`}
    >
      <span className="text-[13px] leading-none">{hub.glyph}</span>
      <span>{hub.name}</span>
      <HubHealthDot hubName={hub.name} health={health} />
      {health === "checking" ? (
        <Loader2 className="w-3 h-3 animate-spin opacity-60" aria-hidden="true" />
      ) : health === "down" ? (
        // Previously rendered at `opacity-0` like the other glyphs, which meant the one state worth interrupting for was the one state nobody could see.
        <AlertCircle
          className="w-3 h-3"
          style={{ color: "var(--color-error, #ef4444)" }}
          aria-hidden="true"
        />
      ) : (
        <Check className="w-3 h-3 opacity-0" aria-hidden="true" />
      )}
      <span className="font-mono text-[10.5px] text-text-dim/80 tabular-nums">
        {count ?? "—"}
      </span>
    </button>
  );
}

export function SkillHubBar({
  hubFilter,
  onChange,
  counts,
  health,
  totalCount,
}: SkillHubBarProps) {
  const allActive = hubFilter === "all";
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-stretch gap-2">
      <button
        type="button"
        onClick={() => onChange("all")}
        className={`px-2.5 py-1.5 text-[12px] rounded-md border inline-flex items-center gap-1.5 cursor-pointer font-medium transition-colors ${
          allActive
            ? "bg-main border-border-strong text-text-main"
            : "border-border-subtle text-text-main hover:bg-main/40"
        }`}
      >
        <Boxes className="w-3.5 h-3.5" />
        {t("skills.all_hubs")}
        {typeof totalCount === "number" && (
          <span className="font-mono text-[10.5px] text-text-dim/80 tabular-nums">
            {totalCount}
          </span>
        )}
      </button>
      {SKILL_HUBS.map((hub) => (
        <HubPill
          key={hub.id}
          hub={hub}
          active={hubFilter === hub.id}
          count={counts?.[hub.id]}
          health={health?.[hub.id]}
          onClick={() => onChange(hub.id)}
        />
      ))}
    </div>
  );
}

/** Headline tile shown when a specific hub is selected — domain, desc, latency. */
export function SkillHubHeadline({ hub }: { hub: SkillHubId }) {
  const h = getSkillHub(hub);
  if (!h) return null;
  return (
    <div
      className="flex items-center gap-3 rounded-lg p-3 mb-3 flex-wrap"
      style={{
        background: `${h.color}0d`,
        border: `1px solid ${h.color}30`,
      }}
    >
      <div
        className="w-10 h-10 rounded-lg grid place-items-center shrink-0 text-xl"
        style={{
          background: `${h.color}14`,
          border: `1px solid ${h.color}40`,
        }}
      >
        {h.glyph}
      </div>
      <div className="flex-1 min-w-[220px]">
        <div className="flex items-center gap-1.5 text-[13.5px] font-semibold text-text-main">
          {h.name}
          <span className="font-mono text-[10.5px] text-text-dim/80 font-normal">
            {h.domain}
          </span>
        </div>
        <p className="text-[12px] text-text-dim/90 mt-0.5">{h.desc}</p>
      </div>
    </div>
  );
}

/** Copy-to-clipboard install command block. Used in the detail drawer. */
export function SkillInstallCommand({
  hub,
  slug,
  onCopied,
}: {
  hub: SkillHubId;
  slug: string;
  /** Fired only after a confirmed clipboard write, so a caller rendering a "Copied" affordance never shows one for a copy that silently failed. */
  onCopied?: () => void;
}) {
  const { t } = useTranslation();
  const h = getSkillHub(hub);
  if (!h) return null;
  const cmd = h.cli(slug);
  return (
    <div
      className="font-mono text-[11.5px] rounded-md flex items-center gap-2 px-2.5 py-2 overflow-hidden"
      style={{
        background: "var(--color-surface, #1a1b23)",
        border: "1px solid var(--color-border-subtle)",
        color: "var(--color-text-main)",
      }}
    >
      <span className="text-text-dim/60">$</span>
      <span className="flex-1 truncate whitespace-nowrap">{cmd}</span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          // The old guard skipped the write entirely when `navigator.clipboard`
          // was undefined — which is exactly the plain-HTTP LAN case — and then
          // fired `onCopied` anyway. The helper falls back to `execCommand` and
          // reports whether anything landed, so the callback is now gated.
          void copyToClipboard(cmd).then((ok) => {
            if (ok) onCopied?.();
          });
        }}
        className="text-text-dim/70 hover:text-text-main transition-colors p-0.5"
        title={t("skills.copy_command", { defaultValue: "Copy command" })}
        aria-label={t("skills.copy_install_command", { defaultValue: "Copy install command" })}
      >
        <svg
          className="w-3.5 h-3.5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
          <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
        </svg>
      </button>
    </div>
  );
}
