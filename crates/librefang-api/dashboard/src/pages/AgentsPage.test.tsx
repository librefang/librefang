// Tests SystemPromptSection / DescriptionSection / ChannelsSection directly
// — AgentsPage has no render harness (~20 hooks).

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SystemPromptSection, DescriptionSection, ChannelsSection } from "./AgentsPage";
import { usePatchAgent, useSetAgentChannels } from "../lib/mutations/agents";
import { useBindPromptVersionToAgent } from "../lib/mutations/prompts";
import { usePromptVersions, useAgentChannels } from "../lib/queries/agents";

vi.mock("../lib/mutations/agents", () => ({
  usePatchAgent: vi.fn(),
  useSetAgentChannels: vi.fn(),
}));

vi.mock("../lib/mutations/prompts", () => ({
  useBindPromptVersionToAgent: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  usePromptVersions: vi.fn(),
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

const usePatchAgentMock = usePatchAgent as unknown as ReturnType<typeof vi.fn>;
const useBindMock = useBindPromptVersionToAgent as unknown as ReturnType<typeof vi.fn>;
const usePromptVersionsMock = usePromptVersions as unknown as ReturnType<typeof vi.fn>;
const useAgentChannelsMock = useAgentChannels as unknown as ReturnType<typeof vi.fn>;
const useSetAgentChannelsMock = useSetAgentChannels as unknown as ReturnType<typeof vi.fn>;

function renderSection(prompt: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <SystemPromptSection agentId="agent-1" prompt={prompt} />
    </QueryClientProvider>,
  );
}

describe("SystemPromptSection (#6187)", () => {
  let patchMutate: ReturnType<typeof vi.fn>;
  let bindMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    patchMutate = vi.fn();
    bindMutate = vi.fn();
    usePatchAgentMock.mockReturnValue({ mutate: patchMutate, isPending: false });
    useBindMock.mockReturnValue({ mutate: bindMutate, isPending: false });
    usePromptVersionsMock.mockReturnValue({ data: [], isLoading: false, isError: false });
  });

  it("Save is disabled until the prompt is edited", () => {
    renderSection("original prompt");
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).toBeDisabled();
  });

  it("editing the prompt and saving PATCHes system_prompt", () => {
    renderSection("original prompt");
    const textarea = screen.getByRole("textbox");
    expect(textarea).toHaveValue("original prompt");
    fireEvent.change(textarea, { target: { value: "updated prompt" } });
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).not.toBeDisabled();
    fireEvent.click(save);
    expect(patchMutate).toHaveBeenCalledTimes(1);
    expect(patchMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      body: { system_prompt: "updated prompt" },
    });
  });

  it("binding a library version calls useBindPromptVersionToAgent with the version", () => {
    const version = {
      id: "ver-1",
      agent_id: "agent-1",
      version: 3,
      content_hash: "abc",
      system_prompt: "library prompt",
      tools: [],
      variables: [],
      created_at: "2025-01-01T00:00:00Z",
      created_by: "tester",
      is_active: false,
    };
    usePromptVersionsMock.mockReturnValue({
      data: [version],
      isLoading: false,
      isError: false,
    });
    renderSection("original prompt");
    fireEvent.click(screen.getByRole("button", { name: /Bind from library/i }));
    fireEvent.click(screen.getByRole("button", { name: /^Bind$/i }));
    expect(bindMutate).toHaveBeenCalledTimes(1);
    expect(bindMutate.mock.calls[0][0]).toEqual({ agentId: "agent-1", version });
  });
});

function renderDescription(description: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <DescriptionSection agentId="agent-1" description={description} />
    </QueryClientProvider>,
  );
}

describe("DescriptionSection (#7742)", () => {
  let patchMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    patchMutate = vi.fn();
    usePatchAgentMock.mockReturnValue({ mutate: patchMutate, isPending: false });
  });

  it("Save is disabled until the description is edited", () => {
    renderDescription("original description");
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).toBeDisabled();
  });

  it("editing the description and saving PATCHes description", () => {
    renderDescription("original description");
    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "updated description" } });
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).not.toBeDisabled();
    fireEvent.click(save);
    expect(patchMutate).toHaveBeenCalledTimes(1);
    expect(patchMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      body: { description: "updated description" },
    });
  });

  it("allows setting a description from empty (the field is no longer hidden when blank)", () => {
    renderDescription("");
    expect(screen.getByRole("textbox")).toHaveValue("");
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
