import { describe, expect, it } from "vitest";
import {
  buildInstalledSlugSet,
  canRollbackSkill,
  combinedQueryHealth,
  filterByCategory,
  isInstalledFromMarketplace,
  isCurrentSkillVersion,
  isRateLimitError,
  tryStartInstall,
} from "./SkillsPage";

describe("SkillsPage marketplace state helpers", () => {
  it("only classifies explicit rate-limit failures", () => {
    expect(isRateLimitError(new Error("request failed with 429"))).toBe(true);
    expect(isRateLimitError({ status: 429 })).toBe(true);
    expect(isRateLimitError(new Error("corporate proxy unavailable"))).toBe(false);
  });

  it("filters any marketplace item list by category metadata", () => {
    const skills = [
      { name: "Deploy helper", description: "Kubernetes operations" },
      { name: "Writing helper", description: "Draft release notes" },
    ];

    expect(filterByCategory(skills, "devops")).toEqual([skills[0]]);
  });

  it("does not treat a local name as an installed ClawHub slug", () => {
    const installed = buildInstalledSlugSet([
      { name: "shared-slug", source: { type: "local" } },
    ]);

    expect(isInstalledFromMarketplace(installed, "shared-slug", "clawhub")).toBe(false);
    expect(isInstalledFromMarketplace(installed, "shared-slug", "skillhub")).toBe(true);
  });

  it("keeps health from both SkillHub query modes", () => {
    expect(
      combinedQueryHealth(
        { isFetching: false, isError: true },
        { isFetching: false, isError: false },
      ),
    ).toBe("down");
    expect(
      combinedQueryHealth(
        { isFetching: false, isError: false },
        { isFetching: true, isError: false },
      ),
    ).toBe("checking");
  });

  it("requires a post-create mutation before rollback", () => {
    const evolution = {
      versions: [
        {
          version: "0.1.0",
          timestamp: "2026-08-15T00:00:00Z",
          changelog: "Initial version",
          content_hash: "hash",
        },
      ],
      use_count: 0,
      evolution_count: 1,
      mutation_count: 0,
    };

    expect(canRollbackSkill(evolution)).toBe(false);
    expect(canRollbackSkill({ ...evolution, mutation_count: 1 })).toBe(true);
  });

  it("identifies the current version by value instead of list position", () => {
    const entry = {
      version: "0.1.0",
      timestamp: "2026-08-15T00:00:00Z",
      changelog: "Initial version",
      content_hash: "hash",
    };

    expect(isCurrentSkillVersion(entry, "0.2.0")).toBe(false);
    expect(isCurrentSkillVersion(entry, "0.1.0")).toBe(true);
  });

  it("rejects install reentry until the active install settles", () => {
    const installing = { current: null as string | null };

    expect(tryStartInstall(installing, "first")).toBe(true);
    expect(tryStartInstall(installing, "second")).toBe(false);
    expect(installing.current).toBe("first");
  });
});
