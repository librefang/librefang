import { useCallback, type KeyboardEvent as ReactKeyboardEvent } from "react";

export interface RovingTabsApi<Id extends string> {
  /** `0` for the roving tab stop (the selected tab), `-1` for the rest. */
  tabIndexFor: (id: Id) => 0 | -1;
  /**
   * Arrow/Home/End handling for one `role="tab"` element, with automatic
   * activation: moving focus moves the selection in the same step. Attach it
   * to every tab, not to the tablist — the target tab is found from the
   * event's own tablist.
   */
  onKeyDown: (event: ReactKeyboardEvent<HTMLElement>, id: Id) => void;
}

/**
 * Keyboard model shared by the dashboard's tab strips (#8597): a roving
 * tabindex plus the APG tabs keys, sized for the strips that carry the tab
 * roles but none of the keyboard behaviour behind them.
 *
 * The hook owns the keys and the tab stop only. Panel wiring (`aria-controls`
 * / `aria-labelledby`) stays in the markup because it differs per site: some
 * strips swap content inside one panel, others mount one panel per tab.
 *
 * Div-based tabs get no native activation from the platform, so callers whose
 * tabs are not `<button>`s handle Enter/Space themselves; buttons already do.
 */
export function useRovingTabs<Id extends string>({
  ids,
  activeId,
  onSelect,
}: {
  /** Tab ids in render order. */
  ids: readonly Id[];
  /** The selected tab. */
  activeId: Id;
  /** Called when an arrow/Home/End moves the selection. */
  onSelect: (id: Id) => void;
}): RovingTabsApi<Id> {
  // Exactly one tab owns the tab stop. Normally the selected one; if the
  // selection is missing from the rendered ids (e.g. it was pruned), anchor
  // on the first tab so the strip is never keyboard-unreachable.
  const rovingId = ids.includes(activeId) ? activeId : ids[0];

  const tabIndexFor = useCallback(
    (id: Id): 0 | -1 => (id === rovingId ? 0 : -1),
    [rovingId],
  );

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLElement>, id: Id) => {
      const index = ids.indexOf(id);
      if (index === -1) return;

      let targetIndex: number;
      switch (event.key) {
        case "ArrowRight":
          targetIndex = (index + 1) % ids.length;
          break;
        case "ArrowLeft":
          targetIndex = (index - 1 + ids.length) % ids.length;
          break;
        case "Home":
          targetIndex = 0;
          break;
        case "End":
          targetIndex = ids.length - 1;
          break;
        default:
          return;
      }
      event.preventDefault();
      const targetId = ids[targetIndex];
      if (targetId === undefined) return;
      if (targetId !== activeId) onSelect(targetId);
      // Move focus directly rather than relying on the re-render: the tab
      // nodes survive the state update (tabs keep their identity), and focus
      // must not depend on the selection round-trip.
      const tabs = event.currentTarget
        .closest('[role="tablist"]')
        ?.querySelectorAll<HTMLElement>('[role="tab"]');
      tabs?.[targetIndex]?.focus();
    },
    [ids, activeId, onSelect],
  );

  return { tabIndexFor, onKeyDown };
}
