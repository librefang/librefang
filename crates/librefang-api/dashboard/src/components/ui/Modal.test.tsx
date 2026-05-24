import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { Modal } from "./Modal";

// `motion/react` ships browser-only animation primitives that jsdom can't
// drive. Same shim as PromptsExperimentsModal.test — render children
// inline and turn `motion.foo` into the corresponding host tag.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
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

describe("Modal autoFocus (#5666)", () => {
  it("panel-right defaults focus to the close button on open", async () => {
    render(
      <Modal isOpen onClose={() => {}} variant="panel-right" title="Panel">
        <input data-testid="first-input" />
        <button>Submit</button>
      </Modal>,
    );

    const closeButton = screen.getByRole("button", { name: "Close" });
    await waitFor(() => expect(document.activeElement).toBe(closeButton));
  });

  it("drawer-right defaults focus to the close button on open", async () => {
    render(
      <Modal isOpen onClose={() => {}} variant="drawer-right" title="Drawer">
        <input data-testid="first-input" />
      </Modal>,
    );

    const closeButton = screen.getByRole("button", { name: "Close" });
    await waitFor(() => expect(document.activeElement).toBe(closeButton));
  });

  it("centred modal defaults focus to the first focusable descendant", async () => {
    render(
      <Modal isOpen onClose={() => {}} title="Centred">
        <input data-testid="first-input" />
        <button>Submit</button>
      </Modal>,
    );

    const input = screen.getByTestId("first-input");
    await waitFor(() => expect(document.activeElement).toBe(input));
  });

  it("autoFocus=\"first\" on panel-right overrides the close-button default", async () => {
    render(
      <Modal
        isOpen
        onClose={() => {}}
        variant="panel-right"
        title="Panel"
        autoFocus="first"
      >
        <input data-testid="first-input" />
      </Modal>,
    );

    const input = screen.getByTestId("first-input");
    await waitFor(() => expect(document.activeElement).toBe(input));
  });

  it("autoFocus=\"close\" with hideCloseButton has no close button to focus and falls back to the first focusable", async () => {
    // Regression guard: when the close button is hidden, the "close"
    // policy must not crash or strand focus on document.body — useFocusTrap's
    // first-focusable fallback should still apply.
    render(
      <Modal
        isOpen
        onClose={() => {}}
        variant="panel-right"
        title="Panel"
        hideCloseButton
        autoFocus="close"
      >
        <input data-testid="first-input" />
      </Modal>,
    );

    const input = screen.getByTestId("first-input");
    await waitFor(() => expect(document.activeElement).toBe(input));
  });
});
