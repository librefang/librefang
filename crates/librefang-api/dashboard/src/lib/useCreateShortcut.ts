import { useEffect, useRef } from "react";
import { CREATE_EVENT } from "./useKeyboardShortcuts";

/// Subscribe to the global `n` keyboard shortcut. When the user presses
/// `n` anywhere outside a text field, the provided handler runs —
/// typically a page's "open new X modal" action.
///
/// Only one handler fires per keypress (the current page's). Unmounting
/// a page removes its listener, so navigating away automatically
/// re-wires `n` to whatever the next page registers.
export function useCreateShortcut(handler: () => void) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    const listener = () => handlerRef.current();
    window.addEventListener(CREATE_EVENT, listener);
    return () => window.removeEventListener(CREATE_EVENT, listener);
  }, []);
}
