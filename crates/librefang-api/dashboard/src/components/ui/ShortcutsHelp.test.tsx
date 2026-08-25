import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { ShortcutsHelp } from "./ShortcutsHelp";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

describe("ShortcutsHelp", () => {
  it("uses the latest close callback for Escape", () => {
    const firstClose = vi.fn();
    const latestClose = vi.fn();
    const { rerender } = render(<ShortcutsHelp isOpen onClose={firstClose} />);

    rerender(<ShortcutsHelp isOpen onClose={latestClose} />);
    fireEvent.keyDown(window, { key: "Escape" });

    expect(firstClose).not.toHaveBeenCalled();
    expect(latestClose).toHaveBeenCalledTimes(1);
  });

  it("constrains the complete dialog and scrolls only its body", () => {
    render(<ShortcutsHelp isOpen onClose={() => {}} />);

    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveClass("max-h-[90vh]", "flex-col");
    expect(dialog.querySelector(".overflow-y-auto")).toHaveClass("min-h-0", "flex-1");
  });
});
