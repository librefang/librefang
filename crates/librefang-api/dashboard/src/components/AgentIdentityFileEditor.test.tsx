import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { AgentIdentityFileEditor } from "./AgentIdentityFileEditor";
import { ApiError } from "../lib/http/errors";
import { useAgentFile } from "../lib/queries/agentFiles";
import { useSetAgentFile } from "../lib/mutations/agentFiles";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, defaultOrOpts?: unknown) => {
      const opts =
        defaultOrOpts && typeof defaultOrOpts === "object"
          ? (defaultOrOpts as Record<string, unknown>)
          : undefined;
      const template =
        opts && "defaultValue" in opts
          ? String(opts.defaultValue)
          : typeof defaultOrOpts === "string"
            ? defaultOrOpts
            : key;
      return opts
        ? template.replace(/\{\{(\w+)\}\}/g, (_m, name: string) =>
            String(opts[name] ?? ""),
          )
        : template;
    },
  }),
}));

const addToast = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToast }) => unknown) =>
    selector({ addToast }),
}));

vi.mock("../lib/queries/agentFiles", () => ({ useAgentFile: vi.fn() }));
vi.mock("../lib/mutations/agentFiles", () => ({ useSetAgentFile: vi.fn() }));

const queryMock = useAgentFile as unknown as ReturnType<typeof vi.fn>;
const saveHookMock = useSetAgentFile as unknown as ReturnType<typeof vi.fn>;

/** The deployed agent file from the parser fixture, so both layers agree on the same bytes. */
const DEPLOYED = `---
name: deannatroi
archetype: assistant
vibe: helpful
emoji:
avatar_url:
greeting_style: warm
color:
---
# Identity
<!-- Visual identity and personality at a glance. Edit these fields freely. -->
`;

let saved: string[] = [];

function mockQuery(overrides: Record<string, unknown> = {}) {
  queryMock.mockReturnValue({
    data: { name: "IDENTITY.md", content: DEPLOYED, size_bytes: DEPLOYED.length },
    isLoading: false,
    isPending: false,
    isError: false,
    error: null,
    ...overrides,
  });
}

function mockSave() {
  saveHookMock.mockReturnValue({
    mutate: (
      content: string,
      options?: { onSuccess?: () => void; onError?: (error: Error) => void },
    ) => {
      saved.push(content);
      options?.onSuccess?.();
    },
    isPending: false,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  saved = [];
  mockQuery();
  mockSave();
});

describe("AgentIdentityFileEditor", () => {
  it("renders the three owned fields with what the file holds", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByLabelText("Archetype")).toHaveProperty("value", "assistant");
    expect(screen.getByLabelText("Vibe")).toHaveProperty("value", "helpful");
    expect(screen.getByLabelText("Greeting style")).toHaveProperty("value", "warm");
  });

  // The heading names what the block holds, and keeps the file's own name in
  // view so the operator can still find it on disk — the file is not renamed.
  it("names the block after the personality, keeping the file name visible", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(
      screen.getByRole("heading", { name: "Personality (IDENTITY.md)" }),
    ).toBeTruthy();
  });

  it("shows the body it does not edit, so the operator can see what is preserved", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(
      screen.getByText(/Visual identity and personality at a glance/),
    ).toBeTruthy();
    expect(screen.getByText(/written back exactly as it is/i)).toBeTruthy();
  });

  // emoji / avatar_url / color belong to the Appearance section, which stores
  // them on the agent record. A second input for a value whose real home is
  // elsewhere is the confusion this panel exists to prevent — so the panel has
  // exactly three text controls, and the foreign keys appear as text only.
  it("lists the foreign keys read-only rather than as inputs", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getAllByRole("textbox")).toHaveLength(3);
    expect(screen.getByText("emoji")).toBeTruthy();
    expect(screen.getByText("avatar_url")).toBeTruthy();
    expect(screen.getByText("color")).toBeTruthy();
    expect(screen.getByText("Appearance")).toBeTruthy();
    expect(screen.getByText(/Set in Appearance/i)).toBeTruthy();
  });

  // A generated file carries personality only, so a block of three "empty"
  // rows would describe keys the operator does not have and cannot act on.
  it("omits the appearance block when the file carries none of the keys", () => {
    const personalityOnly = `---\nname: deannatroi\narchetype: assistant\nvibe: helpful\ngreeting_style: warm\n---\n# Identity\n`;
    mockQuery({
      data: {
        name: "IDENTITY.md",
        content: personalityOnly,
        size_bytes: personalityOnly.length,
      },
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.queryByText("emoji")).toBeNull();
    expect(screen.queryByText("avatar_url")).toBeNull();
    expect(screen.queryByText("color")).toBeNull();
    expect(screen.queryByText(/Appearance/i)).toBeNull();
  });

  // An older file keeps whatever it has — an empty value shows as empty, and a
  // key the file does not carry stays out, so the list is the file's contents
  // rather than the set of keys the format allows.
  it("lists only the appearance keys the file carries", () => {
    const partially = `---\nname: deannatroi\narchetype: assistant\nemoji:\ncolor: orange\n---\n# Identity\n`;
    mockQuery({
      data: {
        name: "IDENTITY.md",
        content: partially,
        size_bytes: partially.length,
      },
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByText("emoji")).toBeTruthy();
    expect(screen.getByText("empty")).toBeTruthy();
    expect(screen.getByText("orange")).toBeTruthy();
    expect(screen.queryByText("avatar_url")).toBeNull();
  });


  it("disables the save button until a field changes", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    const save = screen.getByRole("button", { name: "Save IDENTITY.md" });
    expect(save).toHaveProperty("disabled", true);

    fireEvent.change(screen.getByLabelText("Vibe"), {
      target: { value: "friendly" },
    });
    expect(save).toHaveProperty("disabled", false);
  });

  it("saves only the edited key and writes the whole file back", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    fireEvent.change(screen.getByLabelText("Vibe"), {
      target: { value: "friendly" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(saved).toEqual([DEPLOYED.replace("vibe: helpful", "vibe: friendly")]);
  });

  it("keeps the body and the unknown keys when a field is cleared", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    fireEvent.change(screen.getByLabelText("Archetype"), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(saved).toEqual([DEPLOYED.replace("archetype: assistant", "archetype:")]);
    expect(saved[0]).toContain("name: deannatroi");
    expect(saved[0]).toContain(
      "<!-- Visual identity and personality at a glance. Edit these fields freely. -->",
    );
  });

  it("reports a successful save and refreshes the draft from the refetched file", () => {
    render(<AgentIdentityFileEditor agentId="agent-1" />);

    fireEvent.change(screen.getByLabelText("Vibe"), {
      target: { value: "friendly" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(addToast).toHaveBeenCalledWith("IDENTITY.md saved", "success");
    // The hook owns invalidation; the component must not add a second call.
    expect(saveHookMock).toHaveBeenCalledWith("agent-1", "IDENTITY.md");
  });

  it("reports a failed save with the server's message", () => {
    saveHookMock.mockReturnValue({
      mutate: (
        _content: string,
        options?: { onError?: (error: Error) => void },
      ) => options?.onError?.(new Error("403 forbidden")),
      isPending: false,
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);
    fireEvent.change(screen.getByLabelText("Vibe"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(addToast).toHaveBeenCalledWith("403 forbidden", "error");
  });
});

describe("AgentIdentityFileEditor — the daemon's 32 KiB cap", () => {
  // Whatever the server says, the operator should learn the size before the
  // save, not from a 413 whose message the daemon renders as
  // `File too large (max { $max })` — the handler never passes `$max`, so it
  // names no limit and no size.
  const ALREADY_OVER = `---\narchetype: assistant\nvibe: ${"x".repeat(33_000)}\n---\n# Identity\n`;

  it("measures bytes, refuses the save, and says by how much", () => {
    mockQuery({
      data: { name: "IDENTITY.md", content: ALREADY_OVER, size_bytes: ALREADY_OVER.length },
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);
    fireEvent.change(screen.getByLabelText("Archetype"), {
      target: { value: "researcher" },
    });

    expect(screen.getByRole("alert").textContent).toMatch(
      /the daemon accepts at most 32768/,
    );
    const save = screen.getByRole("button", { name: "Save IDENTITY.md" });
    expect(save).toHaveProperty("disabled", true);

    fireEvent.click(save);
    expect(saved).toEqual([]);
  });

  // A file can only be over the cap if something wrote it outside the API, and
  // then the dashboard cannot save it back at all. Saying so beats letting the
  // operator edit and fail.
  it("warns that an already-oversized file cannot be saved, before anything is typed", () => {
    mockQuery({
      data: { name: "IDENTITY.md", content: ALREADY_OVER, size_bytes: ALREADY_OVER.length },
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByRole("alert").textContent).toMatch(
      /the daemon accepts at most 32768/,
    );
    expect(screen.getByRole("button", { name: "Save IDENTITY.md" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("counts bytes rather than characters, so a CJK file is not waved through", () => {
    // 12 000 three-byte characters is 36 000 bytes but only 12 000 characters:
    // a character count would pass this file and the daemon would reject it.
    const cjk = `---\nvibe: ${"漢".repeat(12_000)}\n---\n`;
    mockQuery({ data: { name: "IDENTITY.md", content: cjk, size_bytes: cjk.length } });

    render(<AgentIdentityFileEditor agentId="agent-1" />);
    fireEvent.change(screen.getByLabelText("Archetype"), { target: { value: "a" } });

    expect(screen.getByRole("button", { name: "Save IDENTITY.md" })).toHaveProperty(
      "disabled",
      true,
    );
  });
});

describe("AgentIdentityFileEditor — file states", () => {
  it("offers to create a file the daemon reports as missing", () => {
    mockQuery({
      data: undefined,
      error: new ApiError(404, "HTTP_404", "not found"),
      isError: true,
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByText(/has no IDENTITY\.md yet/)).toBeTruthy();
    expect(screen.getByLabelText("Vibe")).toHaveProperty("value", "");

    fireEvent.change(screen.getByLabelText("Vibe"), { target: { value: "warm" } });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(saved).toEqual(["---\nvibe: warm\n---\n"]);
  });

  it("prepends a block to a file that has no front-matter, keeping the body", () => {
    const content = "# Identity\n\nSome prose.\n";
    mockQuery({ data: { name: "IDENTITY.md", content, size_bytes: content.length } });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByText(/has no front-matter block/)).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Archetype"), {
      target: { value: "assistant" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save IDENTITY.md" }));

    expect(saved).toEqual([
      "---\narchetype: assistant\n---\n# Identity\n\nSome prose.\n",
    ]);
  });

  it("goes read-only when the front-matter block is not closed", () => {
    const content = "---\nvibe: warm\n# Identity\n";
    mockQuery({ data: { name: "IDENTITY.md", content, size_bytes: content.length } });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.getByRole("alert").textContent).toMatch(/never closes it/);
    expect(screen.queryByLabelText("Vibe")).toBeNull();
    expect(screen.queryByRole("button", { name: "Save IDENTITY.md" })).toBeNull();
  });

  it("shows a read failure that is not a missing file as an error", () => {
    mockQuery({
      data: undefined,
      error: new ApiError(500, "HTTP_500", "workspace error"),
      isError: true,
    });

    render(<AgentIdentityFileEditor agentId="agent-1" />);

    expect(screen.queryByLabelText("Vibe")).toBeNull();
    expect(screen.getByText("workspace error")).toBeTruthy();
    expect(screen.queryByText(/has no IDENTITY\.md yet/)).toBeNull();
  });
});
