import { useCallback, useEffect, useRef, useState } from "react";

// Skip when the user is typing into a form field — same rule as the
// global g-nav shortcuts.
function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (target.isContentEditable) return true;
  return false;
}

export interface ListNavOptions<T> {
  // Items currently rendered in the list (filtered, sorted, etc.).
  items: readonly T[];
  // Called on Enter — usually opens detail / drills in.
  onActivate?: (item: T, index: number) => void;
  // Called on Esc — usually closes detail or clears selection.
  onEscape?: () => void;
  // Disable the listener entirely (e.g., when a modal owns the keyboard).
  disabled?: boolean;
}

export interface ListNavItemProps {
  ref: (el: HTMLElement | null) => void;
  tabIndex: number;
  "aria-selected": boolean;
  "data-listnav-index": number;
  onMouseEnter: () => void;
  onClick: () => void;
}

export interface ListNavApi {
  selectedIndex: number;
  setSelectedIndex: (idx: number) => void;
  getItemProps: (index: number) => ListNavItemProps;
}

/**
 * Vim-style keyboard navigation for any list:
 *  - `j` / `↓` — next
 *  - `k` / `↑` — prev
 *  - `g g` — top
 *  - `G` (shift) — bottom
 *  - `Enter` — onActivate(items[selectedIndex])
 *  - `Esc` — onEscape() (and clear selection)
 *
 * Selection is index-based and clamped against `items.length`. The hook
 * silently no-ops while focus is inside an input / textarea / select /
 * contenteditable so that typing isn't hijacked.
 *
 * Usage:
 *   const nav = useListNav({ items, onActivate: (a) => openDetail(a.id) });
 *   {items.map((a, i) => (
 *     <Card {...nav.getItemProps(i)} key={a.id}>...</Card>
 *   ))}
 */
export function useListNav<T>({
  items,
  onActivate,
  onEscape,
  disabled,
}: ListNavOptions<T>): ListNavApi {
  const [selectedIndex, setSelectedIndexRaw] = useState(-1);
  const itemRefs = useRef(new Map<number, HTMLElement>());
  const lastGAt = useRef(0);

  const length = items.length;
  // Re-clamp when the list shrinks past the current selection.
  useEffect(() => {
    if (selectedIndex >= length) {
      setSelectedIndexRaw(length === 0 ? -1 : length - 1);
    }
  }, [length, selectedIndex]);

  const setSelectedIndex = useCallback((idx: number) => {
    setSelectedIndexRaw(idx);
  }, []);

  // Scroll selected row into view (centered in viewport).
  useEffect(() => {
    if (selectedIndex < 0) return;
    const el = itemRefs.current.get(selectedIndex);
    el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    // Move focus so screen readers announce the new selection.
    if (el && document.activeElement !== el) {
      // Avoid stealing focus from the body when the user hasn't yet
      // engaged with the list — only refocus if the list itself already
      // had focus.
      const inListNav = document.activeElement?.closest("[data-listnav-index]");
      if (inListNav) el.focus({ preventScroll: true });
    }
  }, [selectedIndex]);

  useEffect(() => {
    if (disabled) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (isTypingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (length === 0 && e.key !== "Escape") return;

      // Enter activates current selection (or first item if none selected).
      if (e.key === "Enter") {
        if (!onActivate) return;
        const idx = selectedIndex >= 0 ? selectedIndex : 0;
        if (idx < length) {
          e.preventDefault();
          onActivate(items[idx], idx);
        }
        return;
      }

      if (e.key === "Escape") {
        if (selectedIndex >= 0) setSelectedIndexRaw(-1);
        onEscape?.();
        return;
      }

      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndexRaw((cur) => Math.min(length - 1, cur < 0 ? 0 : cur + 1));
        lastGAt.current = 0;
        return;
      }
      if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndexRaw((cur) => Math.max(0, cur < 0 ? 0 : cur - 1));
        lastGAt.current = 0;
        return;
      }

      // Shift+G → bottom (vim).
      if (e.key === "G" && e.shiftKey) {
        e.preventDefault();
        setSelectedIndexRaw(length - 1);
        lastGAt.current = 0;
        return;
      }

      // gg → top (vim). 1500ms window to match the global g-nav shortcut.
      if (e.key === "g") {
        const now = Date.now();
        if (lastGAt.current && now - lastGAt.current < 1500) {
          e.preventDefault();
          setSelectedIndexRaw(0);
          lastGAt.current = 0;
        } else {
          lastGAt.current = now;
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [items, length, selectedIndex, onActivate, onEscape, disabled]);

  const getItemProps = useCallback(
    (index: number): ListNavItemProps => ({
      ref: (el: HTMLElement | null) => {
        if (el) itemRefs.current.set(index, el);
        else itemRefs.current.delete(index);
      },
      tabIndex: index === selectedIndex ? 0 : -1,
      "aria-selected": index === selectedIndex,
      "data-listnav-index": index,
      onMouseEnter: () => setSelectedIndexRaw(index),
      onClick: () => setSelectedIndexRaw(index),
    }),
    [selectedIndex],
  );

  return { selectedIndex, setSelectedIndex, getItemProps };
}
