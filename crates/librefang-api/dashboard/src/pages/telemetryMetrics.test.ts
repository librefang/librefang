import { describe, expect, it } from "vitest";
import { parseMetrics } from "./telemetryMetrics";

describe("parseMetrics", () => {
  it("parses reordered and extended label sets by name", () => {
    const parsed = parseMetrics(`
librefang_http_requests_total{status="200",extra="kept",path="/api/agents",method="GET"} 12
librefang_tokens_output{model="gpt-5",agent="alpha",provider="openai"} 30
librefang_tool_calls{provider="openai",agent="alpha",model="gpt-5"} 2
librefang_tokens{model="gpt-5",region="us",agent="alpha",provider="openai"} 100
librefang_tokens_input{agent="alpha",provider="openai",model="gpt-5"} 70
librefang_llm_calls{agent="alpha",provider="openai",model="gpt-5"} 4
`);

    expect(parsed.requests).toEqual([
      { method: "GET", path: "/api/agents", status: "200", count: 12 },
    ]);
    expect(parsed.agents).toEqual([
      {
        agent: "alpha",
        provider: "openai",
        model: "gpt-5",
        tokens: 100,
        inputTokens: 70,
        outputTokens: 30,
        toolCalls: 2,
        llmCalls: 4,
      },
    ]);
  });

  it("decodes escaped label values", () => {
    const parsed = parseMetrics(
      String.raw`librefang_http_requests_total{path="/quoted\"path",method="GET",status="404"} 1`,
    );

    expect(parsed.requests[0]?.path).toBe('/quoted"path');
  });

  it("rejects fractional counters and filters zero-only or rollup agents", () => {
    const parsed = parseMetrics(`
librefang_tokens{agent="fractional",provider="openai",model="gpt"} 1.5
librefang_tool_calls{agent="zero"} 0
librefang_tokens{agent="namespace:rollup",provider="openai",model="gpt"} 9
librefang_tokens_input{agent="active"} 3
`);

    expect(parsed.agents).toEqual([
      {
        agent: "active",
        provider: "",
        model: "",
        tokens: 0,
        inputTokens: 3,
        outputTokens: 0,
        toolCalls: 0,
        llmCalls: 0,
      },
    ]);
  });

  it("parses unlabeled gauges and version labels", () => {
    const parsed = parseMetrics(`
librefang_uptime_seconds 42.5
librefang_agents_active 2
librefang_info{commit="abc",version="1.2.3"} 1
`);

    expect(parsed.system.uptime).toBe(42.5);
    expect(parsed.system.agentsActive).toBe(2);
    expect(parsed.system.version).toBe("1.2.3");
  });
});
