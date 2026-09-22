// `UserAppearanceSection` — the emoji / avatar editor in the user form (#8339).
// Tested directly rather than through `UsersPage`, which has ~20 hooks and no
// render harness; that is the same reason `AgentAppearanceSection` is exported.
//
// The file checks below come in pairs on purpose. "The upload was refused" is
// only worth anything next to "the upload happens", because a component that
// never calls the mutation passes the negative half alone.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UserAppearanceSection } from "./UsersPage";
import * as http from "../lib/http/client";
import {
  useDeleteUserAvatar,
  useUpdateUserIdentity,
  useUploadUserAvatar,
} from "../lib/mutations/users";

vi.mock("../lib/mutations/users", () => ({
  useUpdateUserIdentity: vi.fn(),
  useUploadUserAvatar: vi.fn(),
  useDeleteUserAvatar: vi.fn(),
}));

// `importActual` so the section exercises the real `queries/users` module — the
// only thing faked is the byte fetch, which is what decides whether a picture
// exists. A hand-written stub of the whole client would have to invent
// `currentUserAvatarPath`, and then "which path is requested" would be asserted
// against the stub.
vi.mock("../lib/http/client", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/http/client")>("../lib/http/client");
  return { ...actual, fetchAuthenticatedImage: vi.fn() };
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
  "common.remove": "Remove",
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
  // Default to "this user has never uploaded one": the daemon answers the
  // avatar route with a 404, which is the state most of these tests start in.
  vi.mocked(http.fetchAuthenticatedImage).mockRejectedValue(new Error("404"));
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => "blob:test/1"),
    revokeObjectURL: vi.fn(),
  });
  vi.mocked(useUpdateUserIdentity).mockReturnValue({
    mutate: updateIdentity,
    isPending: false,
  } as unknown as ReturnType<typeof useUpdateUserIdentity>);
  vi.mocked(useUploadUserAvatar).mockReturnValue({
    mutate: uploadAvatar,
    isPending: false,
  } as unknown as ReturnType<typeof useUploadUserAvatar>);
  vi.mocked(useDeleteUserAvatar).mockReturnValue({
    mutate: deleteAvatar,
    isPending: false,
  } as unknown as ReturnType<typeof useDeleteUserAvatar>);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const NAME = "alice";

function renderSection(props: { emoji?: string; hasAvatar?: boolean } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <UserAppearanceSection
        name={NAME}
        emoji={props.emoji}
        hasAvatar={props.hasAvatar}
      />
    </QueryClientProvider>,
  );
  return view;
}


function fileInput(): HTMLInputElement {
  return screen.getByTestId("user-avatar-file-input") as HTMLInputElement;
}

/** A `File` that reports a size the test chooses, without allocating it. */
function fileOfSize(name: string, type: string, size: number): File {
  const file = new File([new Uint8Array([1])], name, { type });
  Object.defineProperty(file, "size", { value: size });
  return file;
}

// The buttons and the preview have to reach the same conclusion about whether a
// picture exists. They did not before: the preview reads `hasAvatar === undefined`
// as "not told, so fetch", while the buttons read it as "no picture", so on a
// daemon that predates the field one drew the image and the other offered to
// upload one.
describe("hasPicture", () => {
  it("offers Replace and Remove from the fetched blob when whoami did not say", async () => {
    vi.mocked(http.fetchAuthenticatedImage).mockResolvedValue(
      new Blob(["x"], { type: "image/png" }),
    );
    renderSection({ hasAvatar: undefined });

    expect(await screen.findByRole("button", { name: "Replace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Upload" })).toBeNull();
  });

  // The other polarity, pinned so it cannot drift back: `false` is a daemon
  // saying "nothing to fetch", which is a different statement from silence.
  it("offers Upload and no Remove when whoami says there is no picture", () => {
    renderSection({ hasAvatar: false });

    expect(screen.getByRole("button", { name: "Upload" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
  });
});

describe("emoji editor", () => {
  it("seeds the field from the stored emoji", () => {
    renderSection({ emoji: "🦊" });
    expect(screen.getByLabelText("Emoji")).toHaveValue("🦊");
  });

  it("keeps Save disabled until the draft differs from what is stored", () => {
    renderSection({ emoji: "🦊" });
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦉" } });
    expect(save).toBeEnabled();
  });

  it("PATCHes the emoji against the user's name", () => {
    renderSection({ emoji: "🦊" });

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦉" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updateIdentity).toHaveBeenCalledTimes(1);
    // A user has no colour field, so the payload is the emoji and nothing else.
    expect(updateIdentity.mock.calls[0][0]).toEqual({ name: NAME, emoji: "🦉" });
  });

  it("sends an empty string to clear, because omitting the field would leave it", () => {
    renderSection({ emoji: "🦊" });

    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updateIdentity.mock.calls[0][0].emoji).toBe("");
  });

  it("submits on Enter, but not while an IME is composing", () => {
    renderSection({ emoji: "🦊" });
    const input = screen.getByLabelText("Emoji");
    fireEvent.change(input, { target: { value: "🦉" } });

    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(updateIdentity).not.toHaveBeenCalled();

    fireEvent.keyDown(input, { key: "Enter" });
    expect(updateIdentity).toHaveBeenCalledTimes(1);
  });

  it("says so once the PATCH lands", () => {
    renderSection({ emoji: "🦊" });
    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦉" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    updateIdentity.mock.calls[0][1].onSuccess();

    expect(addToast).toHaveBeenCalledWith("Emoji updated", "success");
  });

  it("surfaces the daemon's own refusal instead of a generic message", () => {
    renderSection({ emoji: "🦊" });
    fireEvent.change(screen.getByLabelText("Emoji"), { target: { value: "🦉" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    // Verbatim from `validate_emoji` in `routes/users.rs` — the length refusal.
    updateIdentity.mock.calls[0][1].onError(
      new Error("emoji must be at most 32 characters; this one is 40"),
    );

    expect(addToast).toHaveBeenCalledWith(
      "emoji must be at most 32 characters; this one is 40",
      "error",
    );
  });

  it("caps the field at the number the daemon enforces", () => {
    renderSection({ emoji: "🦊" });
    // `MAX_EMOJI_CHARS` in `routes/users.rs`. The browser cap is a courtesy;
    // the daemon refuses an over-long value regardless of what gets typed here.
    expect(screen.getByLabelText("Emoji")).toHaveAttribute("maxlength", "32");
  });
});

describe("avatar upload", () => {
  it("offers Upload with no avatar, and Replace plus Remove once there is one", () => {
    const { unmount } = renderSection();
    expect(screen.getByRole("button", { name: "Upload" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
    unmount();

    // The same section, with `whoami` having said there is a picture. The
    // section no longer asks the daemon this itself — it cannot, because the
    // answer is what decides whether asking is worth a request.
    renderSection({ hasAvatar: true });
    expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove" })).toBeInTheDocument();
  });

  it("accepts exactly the four types the daemon stores, and not SVG", () => {
    renderSection();
    expect(fileInput().accept).toBe("image/png,image/jpeg,image/gif,image/webp");
    expect(fileInput().accept).not.toContain("svg");
  });

  it("sends the picked file for this user", () => {
    renderSection();
    const file = fileOfSize("me.png", "image/png", 1024);

    fireEvent.change(fileInput(), { target: { files: [file] } });

    expect(uploadAvatar).toHaveBeenCalledTimes(1);
    expect(uploadAvatar.mock.calls[0][0]).toEqual({ name: NAME, file });
  });

  it("refuses an SVG without spending the upload", () => {
    renderSection();

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
    renderSection();
    // One byte over 2 MiB — the cap the route enforces.
    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("big.png", "image/png", 2 * 1024 * 1024 + 1)] },
    });

    expect(uploadAvatar).not.toHaveBeenCalled();
    expect(addToast).toHaveBeenCalledWith("That image is 2.0 MB; the limit is 2 MB.", "error");
  });

  it("accepts a file exactly at the cap", () => {
    renderSection();

    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("exact.png", "image/png", 2 * 1024 * 1024)] },
    });

    expect(uploadAvatar).toHaveBeenCalledTimes(1);
  });

  it("clears the input so the same file can be picked again after a rejection", () => {
    renderSection();
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

  it("says so once the upload lands", () => {
    renderSection();
    fireEvent.change(fileInput(), {
      target: { files: [fileOfSize("me.png", "image/png", 10)] },
    });

    uploadAvatar.mock.calls[0][1].onSuccess();

    expect(addToast).toHaveBeenCalledWith("Avatar updated", "success");
  });
});

describe("avatar removal", () => {
  it("does not offer a removal for a user who has no picture", () => {
    renderSection();

    // No Remove button is drawn until a picture exists, so drive the removal
    // through the same path the button uses rather than a hidden one.
    expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
    expect(deleteAvatar).not.toHaveBeenCalled();
  });

  it("deletes by name once there is a picture to remove", () => {
    renderSection({ hasAvatar: true });
    const remove = screen.getByRole("button", { name: "Remove" });

    fireEvent.click(remove);
    expect(deleteAvatar.mock.calls[0][0]).toBe(NAME);

    deleteAvatar.mock.calls[0][1].onSuccess();
    expect(addToast).toHaveBeenCalledWith("Avatar removed", "success");
  });
});
