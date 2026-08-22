import { Plus, Sparkles, X } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * Single skill row rendered inside the agent detail panel's Skills tab.
 *
 * Issue #4925: the assignment UI used to show only the skill name, so
 * users with 40+ skills had no clue what `web-search` vs `web-research`
 * vs `web-fetch` actually do. We cross-reference the global skill
 * registry (`useSkills()` in AgentsPage) and pass the description in
 * here so each row shows the human-readable summary inline below the
 * name. When no description is available (skill not in the global list,
 * or its `description` field is empty) we fall back to the previous
 * "installed" hint so the row still has a stable second line and the
 * grid layout doesn't jump.
 *
 * Issue #4917: the tab gained inline assignment. The same row now serves
 * two roles via the `action` prop:
 *   - `"remove"` (assigned skills) — a trailing ✕ button that unassigns.
 *   - `"add"` (available, not-yet-assigned skills) — a trailing ＋ button
 *     that assigns. The whole row is the click target in this mode so the
 *     hit area matches the old "click to open" affordance.
 * With no `action` (or `"none"`) the row is the read-only display used by
 * the "all" informational list.
 *
 * Extracted into its own component so it can be unit-tested without
 * mounting the entire AgentsPage (which pulls in routing, multiple
 * queries, the manifest form, etc.).
 */
export interface AgentSkillItemProps {
  name: string;
  description?: string;
  /**
   * Row click handler. In `"add"` mode this is the assign action (the whole
   * row is clickable); in `"remove"` / display mode it is the optional
   * navigate-to-detail affordance.
   */
  onClick?: () => void;
  /** Trailing-affordance variant. Defaults to `"none"` (display only). */
  action?: "none" | "add" | "remove";
  /** Click handler for the trailing ✕ in `"remove"` mode. */
  onRemove?: () => void;
  /** Disable the trailing affordance while a mutation is in flight. */
  busy?: boolean;
}

export function AgentSkillItem({
  name,
  description,
  onClick,
  action = "none",
  onRemove,
  busy = false,
}: AgentSkillItemProps) {
  const { t } = useTranslation();
  const trimmedDescription = description?.trim();
  const subtitle = trimmedDescription || t("agents.detail.skill_meta", {
    defaultValue: "installed",
  });
  const content = (
    <>
      <div className="min-w-0 flex-1">
        <div
          className="font-mono text-[12.5px] font-medium text-text-main truncate"
          data-testid="agent-skill-item-name"
        >
          {name}
        </div>
        <div
          className="font-mono text-[10.5px] text-text-dim/80 mt-0.5 line-clamp-2"
          data-testid="agent-skill-item-description"
          title={trimmedDescription || undefined}
        >
          {subtitle}
        </div>
      </div>
      {action === "add" && (
        <Plus
          className="w-3.5 h-3.5 text-brand/70 shrink-0 mt-0.5"
          data-testid="agent-skill-item-add"
        />
      )}
      {action === "none" && (
        <Sparkles className="w-3.5 h-3.5 text-brand/70 shrink-0 mt-0.5" />
      )}
    </>
  );

  return (
    <div
      className={`rounded-md border border-border-subtle bg-main/40 transition-colors flex items-stretch ${
        onClick ? "cursor-pointer hover:border-brand/40" : ""
      } ${busy ? "opacity-50" : ""}`}
      aria-busy={busy || undefined}
      data-testid="agent-skill-item"
    >
      {onClick ? (
        <button
          type="button"
          onClick={onClick}
          disabled={busy}
          className="flex min-w-0 flex-1 items-start justify-between gap-2 px-3 py-2.5 text-left disabled:cursor-not-allowed"
          data-testid="agent-skill-item-action"
        >
          {content}
        </button>
      ) : (
        <div
          className="flex min-w-0 flex-1 items-start justify-between gap-2 px-3 py-2.5"
        >
          {content}
        </div>
      )}
      {action === "remove" && (
        <button
          type="button"
          onClick={(e) => {
            // Don't let the click bubble to the row's onClick (navigate).
            e.stopPropagation();
            onRemove?.();
          }}
          disabled={busy}
          aria-label={t("agents.detail.skill_remove", {
            defaultValue: "Remove {{name}}",
            name,
          })}
          title={t("agents.detail.skill_remove", {
            defaultValue: "Remove {{name}}",
            name,
          })}
          className="mr-3 mt-3 shrink-0 self-start rounded p-0.5 text-text-dim hover:text-red-400 hover:bg-red-400/10 transition-colors disabled:opacity-50"
          data-testid="agent-skill-item-remove"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  );
}
