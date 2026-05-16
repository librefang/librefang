import type { KeyboardEvent } from "react";
import { Sparkles } from "lucide-react";
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
 * Extracted into its own component so it can be unit-tested without
 * mounting the entire AgentsPage (which pulls in routing, multiple
 * queries, the manifest form, etc.).
 */
export interface AgentSkillItemProps {
  name: string;
  description?: string;
  onClick?: () => void;
}

export function AgentSkillItem({ name, description, onClick }: AgentSkillItemProps) {
  const { t } = useTranslation();
  const trimmedDescription = description?.trim();
  const subtitle = trimmedDescription && trimmedDescription.length > 0
    ? trimmedDescription
    : t("agents.detail.skill_meta", { defaultValue: "installed" });
  // The row is interactive when an onClick is provided. Mirror that to
  // keyboard + assistive tech with role/tabIndex/Enter+Space handling so
  // it isn't a mouse-only target.
  const handleKeyDown = onClick
    ? (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onClick();
        }
      }
    : undefined;
  return (
    <div
      onClick={onClick}
      onKeyDown={handleKeyDown}
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      className="px-3 py-2.5 rounded-md border border-border-subtle bg-main/40 cursor-pointer hover:border-brand/40 transition-colors flex items-start justify-between gap-2"
      data-testid="agent-skill-item"
    >
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
      <Sparkles className="w-3.5 h-3.5 text-brand/70 shrink-0 mt-0.5" />
    </div>
  );
}
