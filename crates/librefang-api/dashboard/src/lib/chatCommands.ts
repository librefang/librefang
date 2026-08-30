// Projection of the server-owned slash-command catalog onto what the chat UI
// needs.
//
// The catalog comes from `GET /api/commands`, which serves the
// `Scope::DASHBOARD` slice of `librefang_channels::commands::COMMAND_REGISTRY`.
// ChatPage used to hard-code the same list, which is how `/goal` ended up
// working in Telegram and nowhere else (upstream #3355).

import type { TFunction } from "i18next";
import type { ChatCommand } from "../api";

/** Commands the chat can actually run, and therefore offers in the slash menu.
 *
 *  Entries without `exec` are catalogued but have no dashboard execution path
 *  (skill commands, and builtins the WS handler does not answer). They stay out
 *  of the menu and fall through to the agent as ordinary text — the behaviour
 *  the hard-coded list had. */
export function menuCommands(commands: ChatCommand[] | undefined): ChatCommand[] {
  return (commands ?? []).filter(c => c.exec === "client" || c.exec === "backend");
}

/** Bare names (no leading slash) that must be dispatched as a
 *  `{"type":"command"}` WebSocket frame rather than sent to the agent. */
export function backendCommandNames(commands: ChatCommand[] | undefined): string[] {
  return (commands ?? []).filter(c => c.exec === "backend").map(c => c.cmd.slice(1));
}

/** Whether a send must be held back because the command catalog has not
 *  arrived yet.
 *
 *  Without it, a slash command typed in that window reaches the agent as a
 *  plain prompt — the list that identifies it as a command is still in
 *  flight — which spends tokens and answers nonsense. The hard-coded array
 *  this module replaced was available on the first frame, so holding the send
 *  is what preserves that guarantee without keeping a second copy of the
 *  catalog around.
 *
 *  A *failed* fetch resolves `commandsPending` to false and is deliberately
 *  not held: with no catalog we cannot tell a builtin from a skill command,
 *  and skill commands are meant to reach the agent. */
export function shouldHoldSlashSend(message: string, commandsPending: boolean): boolean {
  return commandsPending && message.trim().startsWith("/");
}

/** Menu label: the locale string when the catalog's `desc_key` resolves, else
 *  the server-supplied English description. The fallback is what lets a newly
 *  registered command render a sensible label before its key is translated. */
export function commandLabel(t: TFunction, c: ChatCommand): string {
  return c.desc_key ? t(`chat.${c.desc_key}`, { defaultValue: c.desc }) : c.desc;
}
