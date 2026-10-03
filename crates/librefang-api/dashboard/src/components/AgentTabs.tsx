import type { LucideIcon } from "lucide-react";

/**
 * One tab of an agent surface: an id to switch on, the label to show, and the
 * icon that makes the bar scannable.
 *
 * The id is generic rather than a union so the same bar draws the two main
 * tabs, the "logs & info" sub-tabs and the config groups. Every one of those
 * lists is derived from a runtime array in `AgentsPage` (or a `Record` keyed by
 * one), which is what lets the guards check the bar against the map instead of
 * against a second hand-written copy of it.
 */
export interface AgentTabDef<Id extends string> {
  id: Id;
  label: string;
  Icon: LucideIcon;
}

/**
 * The tab bar for an agent surface.
 *
 * Pulled out of `AgentsPage` so the join between the layout maps and the
 * surface can be tested at all: the page mounts some twenty hooks and has no
 * render harness, so a bar left inline is unverifiable, and a bar that draws
 * nothing for one group looks exactly like a group with nothing in it.
 *
 * `underline` is the tab-strip look the agent detail panel already uses;
 * `pill` is the segmented look the prompts panel uses for its two internal
 * tabs. Two variants rather than a style prop because there are exactly two
 * visual roles here, and a caller that could pass arbitrary classes would
 * drift into a third.
 */
export function AgentTabBar<Id extends string>({
  tabs,
  active,
  onSelect,
  ariaLabel,
  variant = "underline",
  className = "",
}: {
  tabs: ReadonlyArray<AgentTabDef<Id>>;
  active: Id;
  onSelect: (id: Id) => void;
  /** Accessible name of the tab list, e.g. "Agent configuration sections". */
  ariaLabel: string;
  variant?: "underline" | "pill";
  className?: string;
}) {
  const isUnderline = variant === "underline";
  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className={
        // `flex-wrap` rather than a horizontal scrollbar: the config bar has
        // eight groups whose labels do not fit on one row at any ordinary
        // width, and a group that is only reachable by scrolling a strip with
        // no visible affordance is a group the operator will not find.
        isUnderline
          ? `flex flex-wrap gap-1 border-b border-border-subtle ${className}`
          : `flex flex-wrap gap-1 ${className}`
      }
    >
      {tabs.map((tab) => {
        const selected = tab.id === active;
        const Icon = tab.Icon;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => onSelect(tab.id)}
            className={
              isUnderline
                ? `px-3 py-2 text-[12.5px] flex items-center gap-1.5 border-b-2 -mb-px shrink-0 transition-colors cursor-pointer ${
                    selected
                      ? "border-brand text-text-main font-medium"
                      : "border-transparent text-text-dim hover:text-text-main"
                  }`
                : `px-3 py-1.5 rounded-lg text-xs font-bold transition-colors flex items-center gap-1.5 ${
                    selected ? "bg-brand text-white" : "bg-main text-text-dim hover:text-text-main"
                  }`
            }
          >
            <Icon className={isUnderline ? "w-[13px] h-[13px]" : "w-3 h-3"} />
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
