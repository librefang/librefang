import { fireEvent, render, screen } from "@testing-library/react";
import { useNavigate } from "@tanstack/react-router";
import { describe, expect, it, vi } from "vitest";
import { CommandPalette, useCommandPalette } from "./CommandPalette";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function ShortcutHarness() {
  const { isOpen } = useCommandPalette();
  return <span>{isOpen ? "open" : "closed"}</span>;
}

describe("CommandPalette", () => {
  it("moves DOM focus with the visual arrow-key selection", () => {
    vi.mocked(useNavigate).mockReturnValue(vi.fn());
    render(<CommandPalette isOpen onClose={vi.fn()} />);

    expect(screen.getByPlaceholderText("command_palette.search_placeholder"))
      .toHaveFocus();

    fireEvent.keyDown(window, { key: "ArrowDown" });
    expect(document.activeElement).toHaveTextContent("nav.workflows");

    fireEvent.keyDown(window, { key: "ArrowUp" });
    expect(document.activeElement).toHaveTextContent("nav.overview");
  });

  it("executes the focused command with Enter", () => {
    const navigate = vi.fn();
    const onClose = vi.fn();
    vi.mocked(useNavigate).mockReturnValue(navigate);
    render(<CommandPalette isOpen onClose={onClose} />);

    fireEvent.keyDown(window, { key: "ArrowDown" });
    fireEvent.keyDown(window, { key: "Enter" });

    expect(navigate).toHaveBeenCalledWith({ to: "/workflows" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not attach its command key handler while closed", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");
    vi.mocked(useNavigate).mockReturnValue(vi.fn());

    render(<CommandPalette isOpen={false} onClose={vi.fn()} />);

    expect(
      addEventListener.mock.calls.filter(([event]) => event === "keydown"),
    ).toHaveLength(0);
    addEventListener.mockRestore();
  });
});

describe("useCommandPalette", () => {
  it("toggles the palette with repeated Ctrl+K shortcuts", () => {
    render(<ShortcutHarness />);

    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(screen.getByText("open")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(screen.getByText("closed")).toBeInTheDocument();
  });
});
