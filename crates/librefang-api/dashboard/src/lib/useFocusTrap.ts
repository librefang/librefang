import { useEffect, useRef } from "react";

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(", ");

/// Traps Tab / Shift+Tab focus movement within the given container while
/// `isOpen` is true, restores focus to the previously-active element on
/// close, and autofocuses the first focusable element inside the
/// container on open.
///
/// Usage:
///   const ref = useRef<HTMLDivElement>(null);
///   useFocusTrap(isOpen, ref);
///   return <div ref={ref}>...</div>;
///
/// Keyboard a11y: ensures users navigating by keyboard can't Tab out of
/// a modal accidentally and lose context, and puts focus back on the
/// button that opened the modal when they close it.
export function useFocusTrap(
  isOpen: boolean,
  containerRef: React.RefObject<HTMLElement | null>,
) {
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const focusableRef = useRef<HTMLElement[]>([]);

  useEffect(() => {
    if (!isOpen) return;

    // Remember which element had focus before the modal opened so we
    // can restore it on close.
    previouslyFocused.current = document.activeElement as HTMLElement | null;

    // Focus the first focusable element inside the modal, falling back
    // to the container itself (needs tabIndex=-1 to receive focus
    // programmatically without joining the tab order).
    const container = containerRef.current;
    if (container) {
      focusableRef.current = Array.from(
        container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (focusableRef.current.length > 0) {
        focusableRef.current[0].focus();
      } else if (container.tabIndex >= -1) {
        container.focus();
      }
      container.setAttribute("aria-modal", "true");
      container.setAttribute("role", "dialog");
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || !container) return;
      const focusable = focusableRef.current.filter(
        (el) => !el.hasAttribute("disabled") && el.getClientRects().length > 0,
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement as HTMLElement | null;
      // Shift+Tab on first → cycle to last; Tab on last → cycle to first.
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      if (container) {
        container.removeAttribute("aria-modal");
        container.removeAttribute("role");
      }
      // Restore focus to the element that opened the modal. Null-guard
      // because the element may have been removed from the DOM (e.g.
      // the page unmounted while the modal was open).
      const target = previouslyFocused.current;
      if (target && document.contains(target)) {
        target.focus();
      }
      previouslyFocused.current = null;
    };
  }, [isOpen, containerRef]);
}
