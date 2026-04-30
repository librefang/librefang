import { useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";

/// Vim-style `g` + letter navigation bindings. The first letter of the
/// route is canonical; collisions (scheduler vs skills, providers vs
/// plugins, memory vs models) fall back to a second distinctive letter.
///
/// Shortcuts are only registered at the App root and only fire when
/// focus is NOT in a text input, textarea, or contenteditable element —
/// otherwise typing the letter "g" while writing a message would
/// hijack the cursor.
export const G_NAV_SHORTCUTS: Record<string, { to: string; label: string }> = {
  o: { to: "/overview", label: "Overview" },
  c: { to: "/chat", label: "Chat" },
  a: { to: "/agents", label: "Agents" },
  h: { to: "/hands", label: "Hands" },
  s: { to: "/skills", label: "Skills" },
  w: { to: "/workflows", label: "Workflows" },
  m: { to: "/models", label: "Models" },
  p: { to: "/providers", label: "Providers" },
  n: { to: "/channels", label: "Channels (Network)" },
  g: { to: "/goals", label: "Goals" },
  l: { to: "/logs", label: "Logs" },
  r: { to: "/runtime", label: "Runtime" },
  e: { to: "/memory", label: "Memory" },
  y: { to: "/analytics", label: "Analytics" },
  t: { to: "/settings", label: "Settings" },
  u: { to: "/plugins", label: "Plugins" },
  k: { to: "/scheduler", label: "Scheduler" },
  v: { to: "/approvals", label: "Approvals (reView)" },
  i: { to: "/sessions", label: "Sessions" },
};

/// Returns true if keyboard input should be ignored because the user is
/// typing into a form field. Checks the active element for INPUT,
/// TEXTAREA, SELECT, or contenteditable.
function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (target.isContentEditable) return true;
  return false;
}

interface KeyboardShortcutsOptions {
  onShowHelp: () => void;
}

/// Custom event dispatched when the user presses `n` — each page can
/// listen for this and open its own "new X" modal. Dispatched at the
/// window level so any component can subscribe.
export const CREATE_EVENT = "librefang:create";

const G_TIMEOUT_MS = 1500;

/// Registers global keyboard shortcuts for the dashboard.
///
/// - `g` + letter → navigate to page (vim-style, see G_NAV_SHORTCUTS)
/// - `?` → open shortcut help modal
/// - `/` → focus the first `[data-shortcut-search]` element on the page
/// - `n` → dispatch CREATE_EVENT so the current page can open its
///         "new X" modal (new agent / new workflow / new skill / ...)
///
/// The `g` prefix state clears after 1500ms if no second key is pressed.
export function useKeyboardShortcuts({ onShowHelp }: KeyboardShortcutsOptions) {
  const navigate = useNavigate();

  useEffect(() => {
    let gPressedAt = 0;

    const handleKeyDown = (e: KeyboardEvent) => {
      // Never intercept keystrokes while the user is typing into a field,
      // or while a modifier key is held (those are reserved for browser
      // / OS shortcuts and cmd-K is handled elsewhere).
      if (isTypingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }

      // `?` opens the help modal. Shift is held to produce `?` on most
      // layouts, so we check the character rather than `e.shiftKey`.
      if (e.key === "?") {
        e.preventDefault();
        onShowHelp();
        return;
      }

      // `/` focuses the first search input on the page (if any).
      if (e.key === "/") {
        const el = document.querySelector<HTMLElement>("[data-shortcut-search]");
        if (el) {
          e.preventDefault();
          el.focus();
          return;
        }
      }

      // `n` dispatches a create event that each page listens for. Pages
      // that have a primary create action (spawn agent, new workflow, ...)
      // subscribe via window.addEventListener(CREATE_EVENT, handler) and
      // open their modal in response.
      if (e.key === "n") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent(CREATE_EVENT));
        return;
      }

      const now = Date.now();

      // If the previous keypress was `g` and still fresh, consume this
      // key as the second half of a `g<x>` navigation binding.
      if (gPressedAt && now - gPressedAt < G_TIMEOUT_MS) {
        gPressedAt = 0;
        const target = G_NAV_SHORTCUTS[e.key.toLowerCase()];
        if (target) {
          e.preventDefault();
          navigate({ to: target.to } as never);
        }
        return;
      }

      if (e.key === "g") {
        gPressedAt = now;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [navigate, onShowHelp]);
}
