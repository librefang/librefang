import { describe, it, expect, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ToolCallsPanel } from "./ToolCallsPanel";
import type { AgentTool } from "../../api";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (opts && typeof opts === "object" && "count" in opts) {
          return `${key}:${opts.count}`;
        }
        return key;
      },
    }),
  };
});

type PanelTool = AgentTool & { _call_id?: string };

function makeTool(overrides: Partial<PanelTool> = {}): PanelTool {
  return {
    name: "shell.exec",
    input: { cmd: "ls" },
    result: "ok",
    running: false,
    is_error: false,
    ...overrides,
  };
}

describe("ToolCallsPanel", () => {
  it("renders nothing when there are no tool calls", () => {
    const { container } = render(<ToolCallsPanel tools={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the total count and the most recent tool name on the bar", () => {
    const tools: PanelTool[] = [
      makeTool({ name: "shell.exec", _call_id: "a" }),
      makeTool({ name: "fs.read", _call_id: "b" }),
    ];
    render(<ToolCallsPanel tools={tools} />);
    const trigger = screen.getByRole("button", { name: /chat\.tool_calls:2/ });
    expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(within(trigger).getByText(/Fs Read/i)).toBeInTheDocument();
  });

  it("surfaces running and error counts when present", () => {
    const tools: PanelTool[] = [
      makeTool({ _call_id: "a", running: true }),
      makeTool({ _call_id: "b", is_error: true, result: "boom" }),
      makeTool({ _call_id: "c" }),
    ];
    render(<ToolCallsPanel tools={tools} />);
    const trigger = screen.getByRole("button", { name: /chat\.tool_calls:3/ });
    // Running badge ("1") and error badge ("1") both render as span text
    // children of the trigger button, so two matches are expected.
    const numericBadges = within(trigger)
      .getAllByText("1", { selector: "span" });
    expect(numericBadges.length).toBe(2);
  });

  it("opens the modal with one ToolCallCard per tool when clicked", async () => {
    const user = userEvent.setup();
    const tools: PanelTool[] = [
      makeTool({ name: "shell.exec", _call_id: "a" }),
      makeTool({ name: "fs.read", _call_id: "b" }),
      makeTool({ name: "memory.recall", _call_id: "c" }),
    ];
    render(<ToolCallsPanel tools={tools} />);
    const trigger = screen.getByRole("button", { name: /chat\.tool_calls:3/ });
    await user.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    const dialog = await screen.findByRole("dialog");
    // Each ToolCallCard renders a header button bearing the prettified tool
    // name. Use a name-based query so we don't depend on internal markup.
    expect(within(dialog).getByRole("button", { name: /Shell Exec/i })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /Fs Read/i })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /Memory Recall/i })).toBeInTheDocument();
  });
});
