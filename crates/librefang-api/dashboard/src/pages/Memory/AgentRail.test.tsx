import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentRail } from "./AgentRail";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? _key,
  }),
}));

const baseProps = {
  agents: [{ id: "agent-123456", name: "Planner" }],
  autoDream: undefined,
  recordsByAgentId: new Map([["agent-123456", 3]]),
  kvCountByAgentId: new Map([["agent-123456", 4]]),
  totalRecords: 7,
  totalKv: 8,
  selectedAgentId: undefined,
  onSelect: vi.fn(),
};

describe("AgentRail", () => {
  it("gives the agent filter a persistent accessible name", () => {
    render(<AgentRail {...baseProps} />);

    const filter = screen.getByRole("textbox", { name: "Filter agents" });
    expect(filter).toHaveAttribute("placeholder", "Filter agents");

    fireEvent.change(filter, { target: { value: "plan" } });
    expect(screen.getByRole("button", { name: "Clear search" })).toBeInTheDocument();
  });

  it("formats aggregate and per-agent memory counts consistently", () => {
    render(<AgentRail {...baseProps} />);

    expect(screen.getByText("7 mem · 8 KV")).toBeInTheDocument();
    expect(screen.getByText("3 mem · 4 KV")).toBeInTheDocument();
  });
});
