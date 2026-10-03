// `AgentAppearanceSection` — the emoji / avatar editor in the agent detail
// drawer (#8339). Tested directly rather than through `AgentsPage`, which has
// ~20 hooks and no render harness; that is the same reason `SystemPromptSection`
// is exported.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AgentAppearanceSection } from "./AgentsPage";
import {
  useDeleteAgentAvatar,
  useUpdateAgentIdentity,
  useUploadAgentAvatar,
} from "../lib/mutations/agents";

vi.mock("../lib/mutations/agents", () => ({
  useUpdateAgentIdentity: vi.fn(),
  useUploadAgentAvatar: vi.fn(),
  useDeleteAgentAvatar: vi.fn(),
}));

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

function renderSection(identity?: { emoji?: string; avatar_url?: string; color?: string }) {
  const onChanged = vi.fn();
  const view = render(
    <AgentAppearanceSection agentId={AGENT} identity={identity} onChanged={onChanged} />,
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
    // an agent the deployment provisions. The dashboard cannot pre-empt it:
    // nothing in the agent payload says whether an agent is provisioned, so
    // relaying the message is the only way the operator learns why.
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
    expect(addToast).toHaveBeenCalledWith("That image is 2.0 MB; the limit is 2 MB.", "error");
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
