import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AgentBrief, hasTokenFootprintData, messageText } from "./AgentBrief";

// Partial mock, not a bare factory: the component's import chain reaches the
// real i18n instance through the store, and a factory that replaced the whole
// module would break that setup rather than only the hook under test.
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: unknown) =>
        opts && typeof opts === "object" && "defaultValue" in (opts as Record<string, unknown>)
          ? (opts as { defaultValue: string }).defaultValue
          : key,
    }),
  };
});

const agent = { id: "agent-1", name: "test-agent", state: "running" };

describe("hasTokenFootprintData", () => {
  it("treats a genuine zero footprint as data (tools-disabled agent, no system_prompt)", () => {
    expect(hasTokenFootprintData(0)).toBe(true);
  });

  it("treats a non-zero footprint as data", () => {
    expect(hasTokenFootprintData(1200)).toBe(true);
  });

  it("treats a missing field as no data", () => {
    expect(hasTokenFootprintData(null)).toBe(false);
    expect(hasTokenFootprintData(undefined)).toBe(false);
  });
});

describe("messageText", () => {
  it("reads a plain string content", () => {
    expect(messageText({ content: "hello" })).toBe("hello");
  });

  it("joins the text blocks of a content array", () => {
    expect(
      messageText({
        content: [
          { type: "text", text: "one" },
          { type: "tool_use" },
          { type: "text", text: "two" },
        ],
      }),
    ).toBe("one two");
  });

  it("reads nothing out of an unknown shape", () => {
    expect(messageText({ content: 42 })).toBe("");
    expect(messageText({})).toBe("");
  });
});

describe("AgentBrief", () => {
  // The footprint is informational and was previously the bottom half of a
  // drawer that had to be opened to read it. It folds here instead — and a
  // folded panel that never renders its body is the same as not having one.
  it("keeps the token footprint folded until the summary is opened", () => {
    render(<AgentBrief agent={{ ...agent, injected_footprint_tokens: 1234 }} />);

    const details = screen.getByText("Token footprint").closest("details");
    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");
    // Both the summary and the body carry the number, which is the point of
    // the folded header: it is readable without opening the panel.
    expect(screen.getAllByText("1,234").length).toBeGreaterThan(0);
  });

  it("omits the footprint panel when the daemon reported no footprint", () => {
    render(<AgentBrief agent={agent} />);
    expect(screen.queryByText("Token footprint")).not.toBeInTheDocument();
  });

  it("lists the recent calls once the footprint is expanded", () => {
    render(
      <AgentBrief
        agent={{ ...agent, injected_footprint_tokens: 10 }}
        events={[
          {
            timestamp: "2026-09-18T10:00:00Z",
            model: "claude-sonnet-5",
            provider: "anthropic",
            input_tokens: 12,
            output_tokens: 3,
            cost_usd: 0.0012,
            tool_calls: 0,
            latency_ms: 900,
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByText("Token footprint"));
    expect(screen.getByText("Recent calls")).toBeInTheDocument();
    expect(screen.getByText("claude-sonnet-5")).toBeInTheDocument();
  });

  // "No conversation yet" and "still loading" are different facts, and the
  // empty state used to show for both.
  it("distinguishes an empty conversation from one still loading", () => {
    const { rerender } = render(
      <AgentBrief agent={agent} conversationLoading hasSession />,
    );
    expect(screen.getByText("Loading...")).toBeInTheDocument();

    rerender(<AgentBrief agent={agent} messages={[]} />);
    expect(screen.getByText(/No conversation yet/)).toBeInTheDocument();
  });

  it("previews at most the last five user and assistant messages", () => {
    const messages = Array.from({ length: 8 }, (_, i) => ({
      role: i % 2 === 0 ? "user" : "assistant",
      content: `m${i}`,
    }));
    render(<AgentBrief agent={agent} messages={messages} />);

    expect(screen.queryByText("m0")).not.toBeInTheDocument();
    expect(screen.queryByText("m2")).not.toBeInTheDocument();
    expect(screen.getByText("m3")).toBeInTheDocument();
    expect(screen.getByText("m7")).toBeInTheDocument();
  });

  it("reports the state, the model and the last activity", () => {
    render(
      <AgentBrief
        agent={{ ...agent, model: { provider: "anthropic", model: "claude-sonnet-5" } }}
      />,
    );
    expect(screen.getAllByText("running").length).toBeGreaterThan(0);
    expect(screen.getByText("claude-sonnet-5")).toBeInTheDocument();
    expect(screen.getByText(/Last activity/)).toBeInTheDocument();
  });
});
