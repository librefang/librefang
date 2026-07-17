import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";
import { TomlViewer } from "./TomlViewer";

// `motion/react` ships browser-only animation primitives jsdom can't drive.
// Same shim as Modal.test / PromptsExperimentsModal.test — render children
// inline and turn `motion.foo` into the corresponding host tag.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
  motion: new Proxy(
    {},
    {
      get:
        (_target: unknown, prop: string) =>
        ({
          children,
          ...rest
        }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string }) =>
        opts?.defaultValue ?? key,
    }),
  };
});

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: () => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

describe("TomlViewer editing (#6478)", () => {
  it("stays read-only when no onSave is provided (no Edit affordance)", () => {
    render(
      <TomlViewer isOpen onClose={() => {}} title="Config" toml={'id = "x"'} />,
    );
    // ConfigPage-style usage: no editing controls.
    expect(screen.queryByText("common.edit")).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("persists edited TOML and leaves edit mode on a successful save", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <TomlViewer
        isOpen
        onClose={() => {}}
        title="HAND.toml"
        toml={'id = "x"'}
        onSave={onSave}
      />,
    );

    fireEvent.click(screen.getByText("common.edit"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe('id = "x"');

    fireEvent.change(textarea, { target: { value: 'id = "x"\nname = "y"' } });
    fireEvent.click(screen.getByText("common.save"));

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith('id = "x"\nname = "y"'),
    );
    // Edit mode closes → the Edit affordance is back, the textarea is gone.
    await waitFor(() =>
      expect(screen.getByText("common.edit")).toBeInTheDocument(),
    );
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("surfaces the 400 validation message and keeps the draft on failure", async () => {
    const onSave = vi
      .fn()
      .mockRejectedValue(new Error("TOML parse error: expected `=`"));
    render(
      <TomlViewer
        isOpen
        onClose={() => {}}
        title="HAND.toml"
        toml={'id = "x"'}
        onSave={onSave}
      />,
    );

    fireEvent.click(screen.getByText("common.edit"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "not valid toml <<>>" } });
    fireEvent.click(screen.getByText("common.save"));

    // The rejection message is shown inline...
    await waitFor(() =>
      expect(
        screen.getByText("TOML parse error: expected `=`"),
      ).toBeInTheDocument(),
    );
    // ...and the editor stays open with the user's draft intact so they can fix it.
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
      "not valid toml <<>>",
    );
  });
});
