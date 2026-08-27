import { describe, expect, it } from "vitest";
import {
  backupsQueryOptions,
  healthDetailQueryOptions,
  queueStatusQueryOptions,
  securityStatusQueryOptions,
  systemStatusQueryOptions,
  taskQueueStatusQueryOptions,
} from "./runtime";

describe("runtime polling policy", () => {
  it("keeps each freshness window aligned with its foreground poll", () => {
    const queries = [
      systemStatusQueryOptions(),
      queueStatusQueryOptions(),
      healthDetailQueryOptions(),
      securityStatusQueryOptions(),
      backupsQueryOptions(),
      taskQueueStatusQueryOptions(),
    ];

    for (const query of queries) {
      expect(query.refetchInterval).toBe(query.staleTime);
      expect(query.refetchIntervalInBackground).toBe(false);
    }
  });
});
