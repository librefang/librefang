// Tests the pure decisions the page makes plus the two surfaces that can be
// rendered without it (ChannelsSection, and the layout guards over the config
// group map). AgentsPage itself has no render harness (~20 hooks).

import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { render, screen, fireEvent, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  canEditAgentIdentity,
  cloneResultNotice,
  createDrawerSeed,
  ChannelsSection,
  CONFIG_GROUPS,
  CONFIG_GROUP_IDS,
  INFO_TABS,
  groupForFirstInvalidField,
} from "./AgentsPage";
import { MANIFEST_SECTION_IDS } from "../components/AgentManifestForm";
import { emptyManifestForm, validateManifestForm } from "../lib/agentManifest";
import { useSetAgentChannels } from "../lib/mutations/agents";
import { useAgentChannels } from "../lib/queries/agents";

vi.mock("../lib/mutations/agents", () => ({
  useSetAgentChannels: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  useAgentChannels: vi.fn(),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) =>
      opts && typeof opts === "object" && "defaultValue" in (opts as Record<string, unknown>)
        ? (opts as { defaultValue: string }).defaultValue
        : key,
    i18n: { language: "en" },
  }),
}));

// Read from disk rather than through the i18n mock above: the mock resolves
// every key to its `defaultValue`, so a missing translation is exactly what it
// cannot see.
const LOCALES_DIR = join(__dirname, "..", "locales");
const LOCALE_FILES = readdirSync(LOCALES_DIR).filter((f) => f.endsWith(".json"));

const useAgentChannelsMock = useAgentChannels as unknown as ReturnType<typeof vi.fn>;
const useSetAgentChannelsMock = useSetAgentChannels as unknown as ReturnType<typeof vi.fn>;

// The daemon gates both appearance writes at `role >= UserRole::Admin`, so the
// floor is the whole content of this predicate: a `user` shown these controls
// collects a 403, and an `admin` denied them is blocked from work the daemon
// would accept.
describe("canEditAgentIdentity", () => {
  it("lets an admin and an owner through", () => {
    expect(canEditAgentIdentity("admin")).toBe(true);
    expect(canEditAgentIdentity("owner")).toBe(true);
  });

  it("turns away the roles that could only collect a 403", () => {
    expect(canEditAgentIdentity("user")).toBe(false);
    expect(canEditAgentIdentity("viewer")).toBe(false);
  });

  // Silence is not permission. `whoami` has not answered on the first render,
  // and reading that as "yes" would flash controls that then disappear.
  it("treats an unanswered whoami as no", () => {
    expect(canEditAgentIdentity(undefined)).toBe(false);
    expect(canEditAgentIdentity("")).toBe(false);
  });
});

// The receiving half of the agent-types Run round trip. AgentsPage has no
// render harness, so this is the only thing pinning the mapping from the
// `template` search param to what the drawer opens on; the param name itself is
// the contract with the sender on /agent-types.
describe("createDrawerSeed", () => {
  it("opens the drawer on the template tab with the named type", () => {
    expect(createDrawerSeed("researcher")).toEqual({
      createMode: "template",
      templateName: "researcher",
    });
  });

  it("opens nothing when the param is absent", () => {
    expect(createDrawerSeed(undefined)).toBeNull();
  });

  // No agent type can be named "", and admitting it would open the drawer on an
  // empty picker with Create disabled and no way back.
  it("opens nothing for an empty value", () => {
    expect(createDrawerSeed("")).toBeNull();
  });
});

describe("cloneResultNotice", () => {
  const base = { agent_id: "agent-copy", name: "copy" };

  it("keeps complete clones on the success path", () => {
    expect(cloneResultNotice({ ...base, partial: false, warnings: [] })).toEqual({
      partial: false,
      warnings: "unknown",
    });
  });

  it("preserves stable warning codes for partial clones", () => {
    expect(cloneResultNotice({
      ...base,
      partial: true,
      warnings: ["identity_files_copy_failed", "registry_identity_copy_failed"],
    })).toEqual({
      partial: true,
      warnings: "identity_files_copy_failed, registry_identity_copy_failed",
    });
  });

  it("fails safe when warnings and the partial flag disagree", () => {
    expect(cloneResultNotice({
      ...base,
      partial: false,
      warnings: ["destination_workspace_missing"],
    }).partial).toBe(true);
  });
});

function renderChannels() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <ChannelsSection agentId="agent-1" />
    </QueryClientProvider>,
  );
}

describe("ChannelsSection (#7742)", () => {
  let setChannelsMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    setChannelsMutate = vi.fn();
    useSetAgentChannelsMock.mockReturnValue({ mutate: setChannelsMutate, isPending: false });
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: ["telegram"], available: ["telegram", "discord", "slack"], mode: "allowlist" },
      isLoading: false,
    });
  });

  it("renders the picker seeded with the assigned channels and no Save button while pristine", () => {
    renderChannels();
    // MultiSelectCmdk swaps its placeholder to "Add more…" once at least
    // one chip is selected (see MultiSelectCmdk.tsx), so with "telegram"
    // already assigned the combobox itself — not the custom placeholder —
    // is the stable query target.
    expect(screen.getByRole("combobox")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove telegram" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /common\.save/i })).not.toBeInTheDocument();
  });

  it("shows the 'no channels configured' message when the instance has none", () => {
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: [], available: [], mode: "all" },
      isLoading: false,
    });
    renderChannels();
    expect(
      screen.getByText("No channels configured on this instance."),
    ).toBeInTheDocument();
  });

  it("still renders an allowlist whose channels are no longer configured on the instance (#7749 review)", async () => {
    // `get_agent_channels` builds `available` from `config.sidecar_channels`
    // alone, so an `agent.toml` carrying `channels = ["telegram"]` after that
    // sidecar channel was removed from `config.toml` reports a non-empty
    // `assigned` against an empty `available`. Gating the picker on
    // `available` hid a live restriction behind "No channels configured" and
    // left no way to clear it.
    const user = userEvent.setup();
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: ["telegram"], available: [], mode: "allowlist" },
      isLoading: false,
    });
    renderChannels();

    expect(
      screen.queryByText("No channels configured on this instance."),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove telegram" })).toBeInTheDocument();

    // …and it is clearable from here, which is the half that mattered.
    await user.click(screen.getByRole("button", { name: "Remove telegram" }));
    fireEvent.click(screen.getByRole("button", { name: /common\.save/i }));
    expect(setChannelsMutate).toHaveBeenCalledTimes(1);
    expect(setChannelsMutate.mock.calls[0][0]).toEqual({ agentId: "agent-1", channels: [] });
  });

  it("picking a channel from the dropdown and saving PUTs the new allowlist (#7742)", async () => {
    const user = userEvent.setup();
    renderChannels();

    const input = screen.getByRole("combobox");
    await user.click(input);
    const list = await screen.findByRole("listbox");
    await user.click(within(list).getByText("discord"));

    const save = screen.getByRole("button", { name: /common\.save/i });
    fireEvent.click(save);

    expect(setChannelsMutate).toHaveBeenCalledTimes(1);
    expect(setChannelsMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      channels: ["telegram", "discord"],
    });
  });
});

describe("agent detail — manifest section layout", () => {
  const hosted = CONFIG_GROUP_IDS.flatMap((group) =>
    CONFIG_GROUPS[group].map((id) => ({ group, id })),
  );

  // A section the form can render but no group hosts is a field the operator
  // cannot reach — the same defect as a manifest key with no widget, one level
  // up, and invisible in every other test because the form would still render
  // it happily wherever it was asked to.
  it("hosts every manifest section somewhere", () => {
    const orphans = MANIFEST_SECTION_IDS.filter(
      (id) => !hosted.some((entry) => entry.id === id),
    );
    expect(
      orphans,
      `Sections exist in the editor that no config group renders, so they ` +
        `are unreachable from the agent view. Add each to CONFIG_GROUPS.\n\n` +
        `Orphaned: ${orphans.join(", ")}`,
    ).toEqual([]);
  });

  // The other direction, and the one that was missing.
  //
  // Everything above asks whether each *real* section is hosted somewhere. None
  // of it asks whether each hosted id is real, so a group naming a section the
  // editor does not implement passed every assertion while rendering a hole:
  // `sections={["skils"]}` finds no match in the form and draws nothing, and
  // the group looks like it loaded an empty tab rather than like it is wrong.
  //
  // The ids are a join between two files — the form implements them, the page
  // arranges them — and a join is exactly where a name has to be checked from
  // both sides.
  it("hosts no section the editor does not implement", () => {
    const known = new Set<string>(MANIFEST_SECTION_IDS);
    const imaginary = hosted.filter((entry) => !known.has(entry.id));

    expect(
      imaginary,
      `CONFIG_GROUPS names sections the editor does not implement, so those ` +
        `groups render nothing where a section should be. Check the id against ` +
        `MANIFEST_SECTION_IDS in AgentManifestForm.tsx.\n\n` +
        `Unknown (${imaginary.length}):\n` +
        imaginary.map((e) => `${e.id} (${e.group})`).join("\n"),
    ).toEqual([]);
  });

  // A section hosted twice is the redundancy the single-surface design exists
  // to remove: two controls writing one field from two places, which is how the
  // complexity router ended up split between a tab and a drawer two levels
  // down.
  it("hosts no manifest section in two groups", () => {
    const seen = new Map<string, string>();
    const clashes: string[] = [];
    for (const { group, id } of hosted) {
      const previous = seen.get(id);
      if (previous) clashes.push(`${id} (${previous} and ${group})`);
      else seen.set(id, group);
    }
    expect(
      clashes,
      `A section rendered by two groups means two controls for one field.\n\n` +
        `Duplicated: ${clashes.join(", ")}`,
    ).toEqual([]);
  });

  // An empty group is a tab that renders nothing, which is the same defect the
  // "unknown section" guard catches from the other side: the operator clicks
  // "Integration" and gets an empty panel with no way to tell it apart from a
  // failed load. The plan listed a group for `metadata`, and `metadata` has no
  // widget anywhere (`manifest-field-coverage.test.ts` records it as
  // preserved-only) — so the group would have shipped empty.
  it("gives every config group at least one section", () => {
    const empty = CONFIG_GROUP_IDS.filter((group) => CONFIG_GROUPS[group].length === 0);

    expect(
      empty,
      `Config groups with no sections render an empty tab. Either give the ` +
        `group a section or drop it from CONFIG_GROUP_IDS.\n\n` +
        `Empty: ${empty.join(", ")}`,
    ).toEqual([]);
  });

  // The group tab bar draws its labels from `agents.group.<id>`, so a group id
  // with no label in the locale files shows the raw key to the operator — and
  // because the bar is built from `CONFIG_GROUP_IDS`, that is a failure the
  // type system cannot see.
  //
  // Checked against every locale rather than against en.json alone: the ids are
  // new, so the four translations are exactly the ones that can go missing
  // while the English surface looks perfect.
  it("labels every config group in every locale", () => {
    const missing: string[] = [];
    for (const file of LOCALE_FILES) {
      const locale = JSON.parse(
        readFileSync(join(LOCALES_DIR, file), "utf8"),
      ) as { agents?: { group?: Record<string, string> } };
      for (const group of CONFIG_GROUP_IDS) {
        if (!locale.agents?.group?.[group]) missing.push(`${file}: agents.group.${group}`);
      }
    }
    expect(
      missing,
      `Group ids are drawn from CONFIG_GROUP_IDS, so every id needs a label ` +
        `in every locale or the tab shows the key itself.\n\n` +
        `Missing (${missing.length}):\n${missing.join("\n")}`,
    ).toEqual([]);
  });

  // The map is free to be reorganised — that is what a map is for — but not
  // every placement is ours to move. `autonomous` is the one the user named:
  // the plan's table omitted it entirely, and grouping the agent's autonomous
  // run settings with the cron jobs and scheduling mode they govern was the
  // decision taken back to them. Pinning that single pair keeps the rest of
  // the map rearranged at will without silently undoing a choice that was not
  // made here.
  it("keeps autonomous in Planning", () => {
    expect(CONFIG_GROUPS.planning).toContain("autonomous");
    const elsewhere = CONFIG_GROUP_IDS.filter(
      (group) => group !== "planning" && CONFIG_GROUPS[group].includes("autonomous"),
    );
    expect(elsewhere, `autonomous also hosted by: ${elsewhere.join(", ")}`).toEqual([]);
  });

  // Same join, one level down: the sub-tabs under "logs & info" are built from
  // `INFO_TABS`, so an id without a label is a tab reading `agents.info.logs`.
  it("labels every logs & info sub-tab in every locale", () => {
    const missing: string[] = [];
    for (const file of LOCALE_FILES) {
      const locale = JSON.parse(
        readFileSync(join(LOCALES_DIR, file), "utf8"),
      ) as { agents?: { info?: Record<string, string> } };
      for (const tab of INFO_TABS) {
        if (!locale.agents?.info?.[tab]) missing.push(`${file}: agents.info.${tab}`);
      }
    }
    expect(
      missing,
      `Sub-tab ids are drawn from INFO_TABS, so every id needs a label in ` +
        `every locale.\n\nMissing (${missing.length}):\n${missing.join("\n")}`,
    ).toEqual([]);
  });
});

// The group jump on a failed save. With the sections grouped, the field that
// failed validation is usually in a group the operator is not looking at — an
// error nobody can see is indistinguishable from no error, and this is the half
// of the layout change that keeps it visible.
//
// Driven by the real validator rather than by hand-written field paths: what
// matters is the pair (validator says X) -> (view goes there), and asserting
// on paths I chose myself would only confirm my idea of what the validator
// emits. Each case trips exactly one rule in an otherwise valid form.
describe("groupForFirstInvalidField", () => {
  const valid = () => {
    const form = emptyManifestForm();
    form.name = "an-agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    return form;
  };

  /** Trips one rule, asserts the validator agrees, and returns the group. */
  function groupFor(mutate: (form: ReturnType<typeof valid>) => void) {
    const form = valid();
    mutate(form);
    const errors = validateManifestForm(form);
    expect(errors.length, `the fixture should trip exactly one rule`).toBeGreaterThan(0);
    return { errors, group: groupForFirstInvalidField(errors) };
  }

  it.each([
    ["a missing name", (f: ReturnType<typeof valid>) => { f.name = ""; }, "general"],
    ["a blank cron", (f: ReturnType<typeof valid>) => { f.schedule = { mode: "periodic", cron: "" }; }, "planning"],
    ["a zero check interval", (f: ReturnType<typeof valid>) => { f.schedule = { mode: "continuous", check_interval_secs: "0" }; }, "planning"],
    ["an out-of-range temperature", (f: ReturnType<typeof valid>) => { f.model.temperature = "9"; }, "model"],
    ["an out-of-range top_p", (f: ReturnType<typeof valid>) => { f.model.top_p = "7"; }, "model"],
    ["an unparseable JSON schema", (f: ReturnType<typeof valid>) => {
      f.response_format = { mode: "json_schema", name: "s", schema: "{not json", strict: false };
    }, "general"],
    ["a shared folder with no path", (f: ReturnType<typeof valid>) => {
      f.workspaces = [{ _uid: "u1", name: "docs", path: "", mode: "rw" }];
    }, "conversation"],
  ])("sends %s to the group that owns the field", (_label, mutate, expected) => {
    const { group } = groupFor(mutate as (form: ReturnType<typeof valid>) => void);
    expect(group).toBe(expected);
  });

  // The first message wins, because the operator is sent to exactly one group
  // and the validator's order is the order the rules are written in.
  it("follows the first error when several are present", () => {
    const form = valid();
    form.name = "";
    form.model.temperature = "9";
    const errors = validateManifestForm(form);
    expect(errors.length).toBeGreaterThan(1);

    // `name` is checked before the model ranges, and identity lives in General
    // while the model lives in Model.
    expect(groupForFirstInvalidField(errors)).toBe("general");
  });

  it("returns nothing when there is nothing to report", () => {
    expect(groupForFirstInvalidField([])).toBeUndefined();
    expect(groupForFirstInvalidField(validateManifestForm(valid()))).toBeUndefined();
  });

  // A path no section claims is already a failure of the coverage guard in
  // AgentManifestForm.test.tsx; this pins the behaviour here so the caller's
  // `if (owningGroup)` fallback is a no-op rather than a crash.
  it("returns nothing for a field path no section claims", () => {
    expect(groupForFirstInvalidField(["campo.inventado"])).toBeUndefined();
  });
});
