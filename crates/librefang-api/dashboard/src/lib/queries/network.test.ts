import { describe, expect, it } from "vitest";
import { networkQueries } from "./network";

describe("networkQueries", () => {
  it("shares one foreground polling policy across live network reads", () => {
    const queries = [
      networkQueries.status(),
      networkQueries.peers(),
      networkQueries.trustedPeers(),
      networkQueries.a2aAgents(),
    ];

    for (const query of queries) {
      expect(query.staleTime).toBe(30_000);
      expect(query.refetchInterval).toBe(15_000);
      expect(query.refetchIntervalInBackground).toBe(false);
    }
  });
});
