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

function isNativeInteractiveOutsideList(): boolean {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return false;
  if (active.closest("[data-listnav-index]")) return false;
  return active.matches(
    "button, a[href], summary, [role='button'], [role='link']",
  );
}

export interface ListNavOptions<T> {
  // Items currently rendered in the list (filtered, sorted, etc.).
  items: readonly T[];
  // Called on Enter — usually opens detail / drills in. Mouse clicks only
  // select; row-local buttons and links retain their own activation behavior.
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
  const itemHandlers = useRef(
    new Map<number, Omit<ListNavItemProps, "tabIndex" | "aria-selected">>(),
  );
  const lastGAt = useRef(0);

  const length = items.length;
  const selectedIndexRef = useRef(selectedIndex);
  const itemsRef = useRef(items);
  const lengthRef = useRef(length);
  const onActivateRef = useRef(onActivate);
  const onEscapeRef = useRef(onEscape);
  selectedIndexRef.current = selectedIndex;
  itemsRef.current = items;
  lengthRef.current = length;
  onActivateRef.current = onActivate;
  onEscapeRef.current = onEscape;

  const setSelectedIndex = useCallback((idx: number) => {
    selectedIndexRef.current = idx;
    setSelectedIndexRaw(idx);
  }, []);

  // Re-clamp when the list shrinks past the current selection.
  useEffect(() => {
    if (selectedIndex >= length) {
      setSelectedIndex(length === 0 ? -1 : length - 1);
    }
  }, [length, selectedIndex, setSelectedIndex]);

  // Scroll selected row into view (centered in viewport).
  useEffect(() => {
    if (selectedIndex < 0) return;
    const el = itemRefs.current.get(selectedIndex);
    el?.scrollIntoView({ block: "nearest", behavior: "auto" });
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
      if (lengthRef.current === 0 && e.key !== "Escape") return;

      // Enter activates current selection (or first item if none selected).
      if (e.key === "Enter") {
        const activate = onActivateRef.current;
        if (!activate || isNativeInteractiveOutsideList()) return;
        const idx = selectedIndexRef.current >= 0 ? selectedIndexRef.current : 0;
        if (idx < lengthRef.current) {
          e.preventDefault();
          activate(itemsRef.current[idx], idx);
        }
        return;
      }

      if (e.key === "Escape") {
        if (selectedIndexRef.current >= 0) setSelectedIndex(-1);
        onEscapeRef.current?.();
        return;
      }

      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        const current = selectedIndexRef.current;
        setSelectedIndex(
          Math.min(lengthRef.current - 1, current < 0 ? 0 : current + 1),
        );
        lastGAt.current = 0;
        return;
      }
      if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        const current = selectedIndexRef.current;
        setSelectedIndex(Math.max(0, current < 0 ? 0 : current - 1));
        lastGAt.current = 0;
        return;
      }

      // Shift+G → bottom (vim).
      if (e.key === "G" && e.shiftKey) {
        e.preventDefault();
        setSelectedIndex(lengthRef.current - 1);
        lastGAt.current = 0;
        return;
      }

      // gg → top (vim). 1500ms window to match the global g-nav shortcut.
      if (e.key === "g") {
        const now = Date.now();
        if (lastGAt.current && now - lastGAt.current < 1500) {
          e.preventDefault();
          setSelectedIndex(0);
          lastGAt.current = 0;
        } else {
          lastGAt.current = now;
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [disabled, setSelectedIndex]);

  const getItemProps = useCallback((index: number): ListNavItemProps => {
    let stable = itemHandlers.current.get(index);
    if (!stable) {
      stable = {
        ref: (el: HTMLElement | null) => {
          if (el) itemRefs.current.set(index, el);
          else itemRefs.current.delete(index);
        },
        "data-listnav-index": index,
        onMouseEnter: () => setSelectedIndex(index),
        // Click selects only. Interactive children retain native behavior;
        // Enter is the explicit list-level activation gesture.
        onClick: () => setSelectedIndex(index),
      };
      itemHandlers.current.set(index, stable);
    }
    return {
      ...stable,
      tabIndex: index === selectedIndexRef.current ? 0 : -1,
      "aria-selected": index === selectedIndexRef.current,
    };
  }, [setSelectedIndex]);

  return { selectedIndex, setSelectedIndex, getItemProps };
}
