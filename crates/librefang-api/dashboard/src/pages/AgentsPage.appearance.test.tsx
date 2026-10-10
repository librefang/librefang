// `AgentAppearanceSection` — the emoji / avatar editor in the agent detail
// drawer (#8339). Tested directly rather than through `AgentsPage`, which has
// ~20 hooks and no render harness; that is the same reason `SystemPromptSection`
// is exported.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentAppearanceSection, AgentsPage } from "./AgentsPage";
import {
  useDeleteAgentAvatar,
  useUpdateAgentIdentity,
  useUploadAgentAvatar,
} from "../lib/mutations/agents";

// The page-level cases at the bottom render the real `AgentsPage`; the
// component cases above render only the section. The mutation module is mocked
// for both, so every hook the page reaches for gets a safe default and the
// three the component tests drive stay bare `vi.fn()`s they can re-arm per
// case.
vi.mock("../lib/mutations/agents", () => {
  const idle = () => ({
    mutate: vi.fn(),
    mutateAsync: vi.fn(),
    isPending: false,
    reset: vi.fn(),
  });
  return {
    useSpawnAgent: () => idle(),
    useCloneAgent: () => idle(),
    useDeleteAgent: () => idle(),
    usePatchAgent: () => idle(),
    useResetAgentSession: () => idle(),
    useResumeAgent: () => idle(),
    useSuspendAgent: () => idle(),
    useUpdateAgentTools: () => idle(),
    useSetAgentSkills: () => idle(),
    useAgentTemplateToml: () => idle(),
    useSetAgentMcpServers: () => idle(),
    useSetAgentChannels: () => idle(),
    useDeleteAgentAvatar: vi.fn(idle),
    useUpdateAgentIdentity: vi.fn(idle),
    useUploadAgentAvatar: vi.fn(idle),
  };
});

const addToast = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToast }) => unknown) =>
    selector({ addToast }),
}));

// The shared buttons are translated by key alone — they predate this section
// and carry no `defaultValue` — so the stub needs their English text for the
// role queries below to read like the UI does.
const SHARED_KEYS: Record<string, string> = {
  "common.save": "Save",
  "common.saving": "Saving...",
};

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => {
      if (opts && typeof opts === "object" && "defaultValue" in (opts as Record<string, unknown>)) {
        const o = opts as Record<string, unknown> & { defaultValue: string };
        // Interpolate the way i18next would, so an assertion on a message can
        // see the numbers that were passed in.
        return o.defaultValue.replace(/\{\{(\w+)\}\}/g, (_m, name: string) =>
          String(o[name] ?? `{{${name}}}`),
        );
      }
      return SHARED_KEYS[key] ?? key;
    },
    i18n: { language: "en" },
  }),
}));

// --- page-level harness ----------------------------------------------------
// Fixtures the `vi.mock` factories below close over have to be hoisted above
// them, and mutable so a case can decide what the daemon returns.
const { agentsRef, whoamiRef } = vi.hoisted(() => ({
  agentsRef: { current: [] as Array<Record<string, unknown>> },
  whoamiRef: { current: { role: "owner" } as { role?: string } | undefined },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useSearch: () => ({}),
  Link: ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
    React.createElement("a", rest, children),
}));

// The agents list is projected from the overview snapshot, not from
// `useAgents` — the page calls `useDashboardSnapshot()` for it.
vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: () => ({
    data: { agents: agentsRef.current },
    isLoading: false,
    isFetching: false,
    refetch: vi.fn(),
  }),
}));
vi.mock("../lib/queries/sessions", () => ({
  useSessionDetails: () => ({ data: undefined, isLoading: false }),
  useSessions: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/memory", () => ({
  useAgentKvMemory: () => ({ data: undefined, isLoading: false }),
  useMemorySearchOrList: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/queries/providers", () => ({
  useProviders: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/models", () => ({
  useModels: () => ({ data: [], isLoading: false, isFetching: false, isError: false, refetch: vi.fn() }),
}));
vi.mock("../lib/queries/skills", () => ({
  useSkills: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/mcp", () => ({
  useMcpServers: () => ({ data: { configured: [] }, isLoading: false }),
}));
vi.mock("../lib/queries/config", () => ({
  useModelRoutingInertReason: () => ({ data: null, isLoading: false }),
}));
vi.mock("../lib/queries/modelRouter", () => ({
  useModelRouterProfiles: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/queries/authz", () => ({
  useWhoami: () => ({ data: whoamiRef.current }),
}));
// The identity section's `IDENTITY.md` editor mounts with the form; it owns
// its own read/write pair, so both sides are stubbed rather than fetched.
vi.mock("../lib/queries/agentFiles", () => ({
  useAgentFile: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/mutations/agentFiles", () => ({
  useSetAgentFile: () => ({ mutate: vi.fn(), isPending: false }),
}));
vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    detail: (id: string) => ({
      queryKey: ["agents", "detail", id],
      queryFn: () =>
        Promise.resolve(agentsRef.current.find((a) => a.id === id) ?? agentsRef.current[0]),
    }),
    manifest: (id: string) => ({
      queryKey: ["agents", "manifest", id],
      queryFn: () => Promise.resolve(undefined),
    }),
  },
  useAgentEvents: () => ({ data: [], isLoading: false }),
  useAgentSessions: () => ({ data: [], isLoading: false }),
  useAgentStats: () => ({ data: undefined, isLoading: false }),
  useAgentTemplates: () => ({ data: [], isLoading: false }),
  useAgentTools: () => ({ data: [], isLoading: false }),
  useAgentSkills: () => ({ data: [], isLoading: false }),
  useAgentMcpServers: () => ({ data: [], isLoading: false }),
  useAgentAvatarUrl: () => ({ data: undefined, isLoading: false }),
  useAgentManifest: () => ({ data: undefined, isLoading: false }),
  useAgentChannels: () => ({ data: [], isLoading: false }),
  useTools: () => ({ data: [], isLoading: false }),
}));

const updateIdentity = vi.fn();
const uploadAvatar = vi.fn();
const deleteAvatar = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(useUpdateAgentIdentity).mockReturnValue({
    mutate: updateIdentity,
    isPending: false,
  } as unknown as ReturnType<typeof useUpdateAgentIdentity>);
  vi.mocked(useUploadAgentAvatar).mockReturnValue({
    mutate: uploadAvatar,
    isPending: false,
  } as unknown as ReturnType<typeof useUploadAgentAvatar>);
  vi.mocked(useDeleteAgentAvatar).mockReturnValue({
    mutate: deleteAvatar,
    isPending: false,
  } as unknown as ReturnType<typeof useDeleteAgentAvatar>);
});

const AGENT = "agent-1";

function renderSection(
  identity?: { emoji?: string; avatar_url?: string; color?: string },
  provisioned?: { source: string } | null,
) {
  const onChanged = vi.fn();
  const view = render(
    <AgentAppearanceSection
      agentId={AGENT}
      identity={identity}
      provisioned={provisioned}
      onChanged={onChanged}
    />,
  );
  return { ...view, onChanged };
}

function fileInput(): HTMLInputElement {
  return screen.getByTestId("agent-avatar-file-input") as HTMLInputElement;
}

/** A `File` that reports a size the test chooses, without allocating it. */
function fileOfSize(name: string, type: string, size: number): File {
  const file = new File([new Uint8Array([1])], name, { type });
  Object.defineProperty(file, "size", { value: size });
  return file;
}

describe("emoji editor", () => {
  it("seeds the field from the stored emoji", () => {
    renderSection({ emoji: "🤖" });
    expect(screen.getByLabelText("Emoji")).toHaveValue("🤖");
  });

  it("keeps Save disabled until the draft differs from what is stored", () => {
    renderSection({ emoji: "🤖" });
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦊" } });
    expect(save).toBeEnabled();
  });

  it("PATCHes only the emoji, so an omitted colour keeps its stored value", () => {
    renderSection({ emoji: "🤖", color: "#ff0000" });

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦊" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updateIdentity).toHaveBeenCalledTimes(1);
    expect(updateIdentity.mock.calls[0][0]).toEqual({
      agentId: AGENT,
      identity: { emoji: "🦊" },
    });
  });

  it("sends an empty string to clear, because omitting the field would leave it", () => {
    renderSection({ emoji: "🤖" });

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updateIdentity.mock.calls[0][0].identity).toEqual({ emoji: "" });
  });

  it("submits on Enter, but not while an IME is composing", () => {
    renderSection({ emoji: "🤖" });
    const input = screen.getByLabelText("Emoji");
    fireEvent.change(input, { target: { value: "🦊" } });

    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(updateIdentity).not.toHaveBeenCalled();

    fireEvent.keyDown(input, { key: "Enter" });
    expect(updateIdentity).toHaveBeenCalledTimes(1);
  });

  it("re-reads the agent and says so once the PATCH lands", () => {
    const { onChanged } = renderSection({ emoji: "🤖" });
    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦊" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    updateIdentity.mock.calls[0][1].onSuccess();

    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith("Emoji updated", "success");
  });

  it("surfaces the server's own message on failure", () => {
    renderSection({ emoji: "🤖" });
    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦊" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    // Verbatim from `guard_provisioned_write` — the 423 the daemon returns for
    // an agent the deployment provisions. A provisioned agent's controls are
    // locked (see below), so this is the fallback for any other server-side
    // refusal: the message is relayed rather than replaced.
    updateIdentity.mock.calls[0][1].onError(
      new Error("this resource is provisioned by the deployment"),
    );

    expect(addToast).toHaveBeenCalledWith(
      "this resource is provisioned by the deployment",
      "error",
    );
  });
});

describe("avatar upload", () => {
  it("offers Upload with no avatar, and Replace plus Remove once there is one", () => {
    const { unmount } = renderSection({});
    expect(screen.getByRole("button", { name: "Upload" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
    unmount();

    renderSection({ avatar_url: `/api/agents/${AGENT}/avatar` });
    expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove" })).toBeInTheDocument();
  });

  it("lets a file with no MIME type through for the daemon to sniff", () => {
    renderSection({});
    // What a browser reports for an extension-less file picked through "All
    // files". The list is a courtesy; the daemon decides by sniffing the
    // bytes, so an empty `type` must not be refused here.
    const file = fileOfSize("picture", "", 1024);

    fireEvent.change(fileInput(), { target: { files: [file] } });

    expect(uploadAvatar).toHaveBeenCalledTimes(1);
    expect(uploadAvatar.mock.calls[0][0]).toEqual({ agentId: AGENT, file });
  });

  it("accepts exactly the four types the daemon stores, and not SVG", () => {
    renderSection({});
    expect(fileInput().accept).toBe("image/png,image/jpeg,image/gif,image/webp");
    expect(fileInput().accept).not.toContain("svg");
  });

  it("sends the picked file", () => {
    renderSection({});
    const file = fileOfSize("me.png", "image/png", 1024);

    fireEvent.change(fileInput(), { target: { files: [file] } });

    expect(uploadAvatar).toHaveBeenCalledTimes(1);
    expect(uploadAvatar.mock.calls[0][0]).toEqual({ agentId: AGENT, file });
  });

  it("refuses an SVG without spending the upload", () => {
    renderSection({});

    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("x.svg", "image/svg+xml", 512)] },
    });

    expect(uploadAvatar).not.toHaveBeenCalled();
    expect(addToast).toHaveBeenCalledWith(
      "An avatar must be a PNG, JPEG, GIF or WebP image. SVG is not accepted.",
      "error",
    );
  });

  it("refuses a file over the cap and names both sizes", () => {
    renderSection({});
    // One byte over 2 MiB — the cap the route enforces.
    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("big.png", "image/png", 2 * 1024 * 1024 + 1)] },
    });

    expect(uploadAvatar).not.toHaveBeenCalled();
    // Rounded up: "2.0 MB; the limit is 2 MB" would contradict itself.
    expect(addToast).toHaveBeenCalledWith("That image is 2.1 MB; the limit is 2 MB.", "error");
  });

  it("accepts a file exactly at the cap", () => {
    renderSection({});

    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("exact.png", "image/png", 2 * 1024 * 1024)] },
    });

    expect(uploadAvatar).toHaveBeenCalledTimes(1);
  });

  it("clears the input so the same file can be picked again after a rejection", () => {
    renderSection({});
    const input = fileInput();

    // Asserting `input.value === ""` afterwards proves nothing: jsdom reports a
    // file input's value as `""` whether or not anything assigned to it, so
    // that check passes against code that never clears. Watch the assignment
    // itself instead — without it the browser fires no `change` for an
    // identical second pick, and a person who re-selects the same file after
    // fixing it sees nothing happen.
    const setValue = vi.fn();
    Object.defineProperty(input, "value", {
      configurable: true,
      get: () => "",
      set: setValue,
    });

    fireEvent.change(input, { target: { files: [fileOfSize("x.svg", "image/svg+xml", 10)] } });

    expect(setValue).toHaveBeenCalledWith("");
  });

  it("re-reads the agent once the upload lands", () => {
    const { onChanged } = renderSection({});
    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("me.png", "image/png", 10)] },
    });

    uploadAvatar.mock.calls[0][1].onSuccess();

    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith("Avatar updated", "success");
  });
});

describe("avatar removal", () => {
  it("deletes by agent id and re-reads the agent", () => {
    const { onChanged } = renderSection({ avatar_url: `/api/agents/${AGENT}/avatar` });

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(deleteAvatar.mock.calls[0][0]).toBe(AGENT);

    deleteAvatar.mock.calls[0][1].onSuccess();
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith("Avatar removed", "success");
  });
});

describe("a provisioned agent", () => {
  const SOURCE = "/etc/librefang/agents/researcher/agent.toml";

  it("locks every control and names the file that owns the appearance", () => {
    renderSection({ emoji: "🤖", avatar_url: `/api/agents/${AGENT}/avatar` }, { source: SOURCE });

    // `guard_provisioned_write` answers 423 to all three writes, so none of
    // them may be offered as if they worked (#8354). The emoji field is the
    // one that is enabled without the gate; the Save button is disabled
    // because the draft is unchanged, and stays disabled.
    expect(screen.getByLabelText("Emoji")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦊" } });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Replace" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove" })).toBeDisabled();
    expect(screen.getByText(new RegExp(SOURCE.replace(/[/.]/g, "\\$&")))).toBeInTheDocument();
  });

  it("does not spend an upload, even if a change event is dispatched anyway", () => {
    renderSection({}, { source: SOURCE });

    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("me.png", "image/png", 1024)] },
    });

    expect(uploadAvatar).not.toHaveBeenCalled();
    expect(updateIdentity).not.toHaveBeenCalled();
    expect(deleteAvatar).not.toHaveBeenCalled();
  });

  it("still renders the stored identity read-only", () => {
    renderSection({ emoji: "🤖", avatar_url: `/api/agents/${AGENT}/avatar` }, { source: SOURCE });

    expect(screen.getByLabelText("Emoji")).toHaveValue("🤖");
    expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
  });

  it("leaves an operator-created agent editable", () => {
    renderSection({ emoji: "🤖" }, null);

    expect(screen.getByLabelText("Emoji")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Upload" })).toBeEnabled();
  });
});

describe("what the editor deliberately does not offer", () => {
  it("has no field for avatar_url", () => {
    renderSection({ avatar_url: `/api/agents/${AGENT}/avatar` });

    // `avatar_url` may only hold this agent's own avatar path (#8349). A
    // free-text box for it would be a way to make the dashboard fetch from
    // wherever the text said, which is what closing the field prevented.
    for (const input of screen.getAllByRole("textbox")) {
      expect(input).toHaveAttribute("aria-label", "Emoji");
    }
    expect(screen.queryByDisplayValue(`/api/agents/${AGENT}/avatar`)).not.toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Page level — the panel's placement in the unified editor, and its two gates.
//
// `AgentManifestForm` is rendered for real here, so the placement assertion
// pins the rendered order inside `general`, not a helper's return value. Only
// the query/mutation layer is mocked.
// ---------------------------------------------------------------------------

const PAGE_AGENT = {
  id: "agent-page-1",
  name: "Alpha",
  is_hand: false,
  state: "running",
  model: { model: "test-model" },
  identity: { emoji: "🤖" },
};

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <AgentsPage />
    </QueryClientProvider>,
  );
}

/** Land on the config tab of the auto-selected agent. */
async function openConfigTab() {
  const tab = await screen.findByRole("tab", { name: "config" });
  fireEvent.click(tab);
  await waitFor(() => expect(tab).toHaveAttribute("aria-selected", "true"));
}

describe("AgentAppearanceSection in the unified editor", () => {
  beforeEach(() => {
    agentsRef.current = [{ ...PAGE_AGENT }];
    whoamiRef.current = { role: "owner" };
    // The auto-select effect only runs on the wide-viewport branch. jsdom has
    // no layout; the setupTests stub answers, and `matches` decides.
    (window.matchMedia("(min-width: 1000px)") as unknown as { matches: boolean }).matches = true;
  });

  it("renders as its own panel above the manifest form in the general group", async () => {
    renderPage();
    await openConfigTab();

    const appearance = screen.getByText("Appearance").closest("section");
    const identitySection = document.querySelector('[data-section="identity"]');
    expect(screen.getByLabelText("Emoji")).toHaveValue("🤖");
    expect(appearance).not.toBeNull();
    expect(identitySection).not.toBeNull();
    // Appearance writes `AgentEntry.identity` through its own endpoint; the
    // form writes the manifest TOML. One store, one writer — the panel is a
    // sibling *before* the form, never a field inside it.
    expect(
      appearance!.compareDocumentPosition(identitySection!) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    // ...and only under `general`.
    fireEvent.click(screen.getByRole("tab", { name: "agents.group.model" }));
    expect(screen.queryByText("Appearance")).toBeNull();
  });

  it("hides the panel from a credential below admin", async () => {
    whoamiRef.current = { role: "user" };
    renderPage();
    await openConfigTab();

    expect(screen.queryByText("Appearance")).toBeNull();
    expect(screen.queryByLabelText("Emoji")).toBeNull();
    // The group itself is unchanged: the form still renders.
    expect(document.querySelector('[data-section="identity"]')).not.toBeNull();
  });

  it("locks the panel in place for a provisioned agent", async () => {
    agentsRef.current = [{ ...PAGE_AGENT, provisioned: { source: "deployment" } }];
    renderPage();
    await openConfigTab();

    // #8354: a provisioned agent's appearance stays visible — controls are
    // disabled and the hint names the source, rather than the panel vanishing.
    expect(screen.getByLabelText("Emoji")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Upload" })).toBeDisabled();
    expect(screen.getByText(/deployment/)).toBeInTheDocument();
  });

  it("withholds the manifest editor from a hand-derived agent (#7835)", async () => {
    agentsRef.current = [{ ...PAGE_AGENT, is_hand: true }];
    renderPage();
    // Hands are hidden from the list by default; the filter reveals the
    // fixture, and the auto-select effect then picks it up.
    fireEvent.click(await screen.findByRole("button", { name: "Hand" }));
    await openConfigTab();

    // A hand's manifest belongs to the Hand definition: the editor would save
    // into agent.toml and the next activation would silently revert it. The
    // notice replaces the form, and the Save/advanced chrome goes with it.
    expect(await screen.findByTestId("manifest-hand-controlled-note")).toBeInTheDocument();
    expect(screen.queryByLabelText("Advanced mode")).toBeNull();
    expect(document.querySelector('[data-section="identity"]')).toBeNull();
  });
});
