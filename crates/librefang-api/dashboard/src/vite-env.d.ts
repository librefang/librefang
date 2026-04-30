/// <reference types="vite/client" />

// xterm.js shipped `cursorInactiveStyle` in 5.5.0 but the
// `@xterm/xterm` type bundle hasn't picked up the field yet (as of
// 5.5.x). Augment the option type so TerminalPage can set it without
// an `as any` cast — values mirror the runtime accepted set.
declare module "@xterm/xterm" {
  interface ITerminalOptions {
    cursorInactiveStyle?: "block" | "underline" | "bar" | "outline" | "none";
  }
}
