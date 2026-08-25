import { describe, expect, it } from "vitest";
import { agentQueries } from "./agents";

describe("agentQueries cache policy", () => {
  it.each([
    ["detail", agentQueries.detail("agent-1")],
    ["templates", agentQueries.templates()],
    ["prompt versions", agentQueries.promptVersions("agent-1")],
    ["experiments", agentQueries.experiments("agent-1")],
    ["experiment metrics", agentQueries.experimentMetrics("experiment-1")],
    ["agent tools", agentQueries.agentTools("agent-1")],
    ["agent skills", agentQueries.agentSkills("agent-1")],
    ["tool list", agentQueries.toolsList()],
  ])("keeps stable %s reads fresh for 30 seconds", (_name, options) => {
    expect(options.staleTime).toBe(30_000);
  });

  it("retains the faster live-event refresh policy", () => {
    const options = agentQueries.events("agent-1");

    expect(options.staleTime).toBe(10_000);
    expect(options.refetchInterval).toBe(15_000);
  });
});
