// DOM ids shared between the terminal strip and the surface it controls
// (#8597). Kept in a module of their own so `TerminalPage` can wire the panel
// without importing from `TerminalTabs` — tests routinely mock that component
// and would lose the constants with it.

/** The xterm surface the window tabs control. */
export const TERMINAL_PANEL_ID = "terminal-panel";

/** DOM id of one window tab; the panel is labelled by the active tab. */
export const terminalTabId = (windowId: string) => `terminal-tab-${windowId}`;
