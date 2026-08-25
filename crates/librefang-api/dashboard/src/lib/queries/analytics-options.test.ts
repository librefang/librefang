import { describe, expect, it } from "vitest";
import { budgetQueries, usageQueries } from "./analytics";

describe("analytics query policy", () => {
  it.each([
    ["usage summary", usageQueries.summary()],
    ["usage by agent", usageQueries.byAgent()],
    ["usage by model", usageQueries.byModel()],
    ["daily usage", usageQueries.daily()],
    ["model performance", usageQueries.modelPerformance()],
    ["budget status", budgetQueries.status()],
    ["provider budgets", budgetQueries.providers()],
  ])("applies the shared foreground polling policy to %s", (_name, options) => {
    expect(options.staleTime).toBe(20_000);
    expect(options.refetchInterval).toBe(30_000);
    expect(options.refetchIntervalInBackground).toBe(false);
  });
});
