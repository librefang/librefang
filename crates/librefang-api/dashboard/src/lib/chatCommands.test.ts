import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import type { ChatCommand } from "../api";
import { backendCommandNames, commandLabel, menuCommands, shouldHoldSlashSend } from "./chatCommands";

// Shape of a real `GET /api/commands` payload, trimmed to the cases that
// matter: a client-resolved builtin, a WS-dispatched builtin, `/goal` (the
// command that used to be missing here — upstream #3355), a catalogued builtin
// with no dashboard execution path, and a skill entry.
const CATALOG: ChatCommand[] = [
  { cmd: "/help", desc: "Show this help", desc_key: "cmd_help", no_args: true, exec: "client" },
  { cmd: "/model", desc: "Show or switch agent model", desc_key: "cmd_model", no_args: false, args_hint: "[name]", exec: "backend" },
  { cmd: "/goal", desc: "Create and start an autonomous goal", desc_key: "cmd_goal", no_args: false, args_hint: "<description> [--loop-engineering]", exec: "backend" },
  { cmd: "/think", desc: "Toggle extended thinking", desc_key: "cmd_think", no_args: false },
  { cmd: "/weather", desc: "Show the current weather", source: "skill" },
];

describe("menuCommands", () => {
  it("offers /goal in the slash menu", () => {
    expect(menuCommands(CATALOG).map(c => c.cmd)).toContain("/goal");
  });

  it("keeps commands with no dashboard execution path out of the menu", () => {
    const offered = menuCommands(CATALOG).map(c => c.cmd);
    expect(offered).not.toContain("/think");
    expect(offered).not.toContain("/weather");
  });

  it("tolerates a catalog that has not loaded yet", () => {
    expect(menuCommands(undefined)).toEqual([]);
  });
});

describe("backendCommandNames", () => {
  it("routes /goal over the WebSocket rather than to the agent", () => {
    expect(backendCommandNames(CATALOG)).toContain("goal");
  });

  it("excludes client-resolved and non-executable commands", () => {
    const backend = backendCommandNames(CATALOG);
    expect(backend).not.toContain("help");
    expect(backend).not.toContain("think");
    expect(backend).not.toContain("weather");
  });

  it("strips the leading slash", () => {
    expect(backendCommandNames(CATALOG).every(name => !name.startsWith("/"))).toBe(true);
  });
});

describe("shouldHoldSlashSend", () => {
  // The regression this guards: moving the catalog to the server introduced a
  // window between mount and the first response in which nothing knew `/goal`
  // was a command, so it would have gone to the agent as a prompt.
  it("holds a slash command typed before the catalog lands", () => {
    expect(shouldHoldSlashSend("/goal ship the release", true)).toBe(true);
  });

  it("holds regardless of leading whitespace", () => {
    expect(shouldHoldSlashSend("   /reset", true)).toBe(true);
  });

  // The /model completion list calls onSend directly, bypassing the input's
  // submit handler, and its own query can resolve before the catalog does.
  // The guard sits on the path every caller funnels through, so this holds
  // too.
  it("holds a command dispatched from the /model completion list", () => {
    expect(shouldHoldSlashSend("/model openai/gpt-4o", true)).toBe(true);
  });

  it("never holds ordinary chat, even while loading", () => {
    expect(shouldHoldSlashSend("what is the status?", true)).toBe(false);
    expect(shouldHoldSlashSend("", true)).toBe(false);
  });

  it("releases the send once the catalog has landed", () => {
    expect(shouldHoldSlashSend("/goal ship the release", false)).toBe(false);
  });

  // A failed fetch also reports `commandsPending: false`. Holding forever
  // would break skill commands, which are supposed to reach the agent.
  it("does not hold when the catalog is unavailable", () => {
    expect(shouldHoldSlashSend("/weather", false)).toBe(false);
  });
});

describe("commandLabel", () => {
  const translate = ((key: string, opts?: { defaultValue?: string }) =>
    key === "chat.cmd_goal" ? "Crear y arrancar un objetivo autónomo" : opts?.defaultValue ?? key) as unknown as TFunction;

  it("prefers the locale string", () => {
    expect(commandLabel(translate, CATALOG[2])).toBe("Crear y arrancar un objetivo autónomo");
  });

  it("falls back to the server description when the key is untranslated", () => {
    expect(commandLabel(translate, CATALOG[1])).toBe("Show or switch agent model");
  });

  it("uses the server description when the entry carries no key (skills)", () => {
    expect(commandLabel(translate, CATALOG[4])).toBe("Show the current weather");
  });
});
