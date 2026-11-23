import { describe, expect, it } from "vitest";
import type { SkillHistoryEntry } from "../api";
import {
  buildInstalledSlugSet,
  canRollbackSkill,
  combinedQueryHealth,
  filterByCategory,
  isInstalledFromMarketplace,
  isRateLimitError,
  marketplaceInstallKey,
  skillHistoryTimestamp,
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
        { isFetching: false, isError: true, isFetched: true },
        { isFetching: false, isError: false, isFetched: false },
      ),
    ).toBe("down");
    expect(
      combinedQueryHealth(
        { isFetching: false, isError: false, isFetched: false },
        { isFetching: true, isError: false, isFetched: false },
      ),
    ).toBe("checking");
    expect(
      combinedQueryHealth(
        { isFetching: false, isError: false, isFetched: true },
        { isFetching: false, isError: false, isFetched: false },
      ),
    ).toBe("live");
  });

  it("reports a hub nobody has contacted as unknown, not live", () => {
    // Both SkillHub queries disabled: no error, no fetch in flight, never resolved (#7387).
    expect(
      combinedQueryHealth(
        { isFetching: false, isError: false, isFetched: false },
        { isFetching: false, isError: false, isFetched: false },
      ),
    ).toBe("unknown");
    expect(combinedQueryHealth()).toBe("unknown");
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

  // The store writes naive UTC in SQLite's `YYYY-MM-DD HH:MM:SS` shape, which
  // `Date` would read as local time if passed through untouched — so the label
  // and the restore dialog would name a version hours off from the server's.
  it("reads the store's naive-UTC timestamp as UTC, not local time", () => {
    const version = { timestamp: "2026-10-14 22:07:53" } as SkillHistoryEntry;

    expect(skillHistoryTimestamp(version)).toBe(
      new Date("2026-10-14T22:07:53Z").toLocaleString(),
    );
  });

  it("rejects install reentry until the active install settles", () => {
    const installing = { current: null as string | null };

    expect(tryStartInstall(installing, "first")).toBe(true);
    expect(tryStartInstall(installing, "second")).toBe(false);
    expect(installing.current).toBe("first");
  });

  it("tells two hubs apart when they publish the same slug", () => {
    // The browse list merges every hub into one grid, and the same slug on
    // two of them is ordinary, not a corner case. Keyed by slug alone, the
    // pending state matched both cards and the wrong one showed as busy.
    expect(marketplaceInstallKey("clawhub", "prd")).not.toBe(
      marketplaceInstallKey("fanghub", "prd"),
    );
  });

  it("matches the identity the browse list already keys its cards by", () => {
    // SkillsPage renders `key={`fanghub:${entry.name}`}`; the pending state has
    // to agree with that or a card is busy according to one identity and idle
    // according to the other.
    expect(marketplaceInstallKey("fanghub", "prd")).toBe("fanghub:prd");
  });
});
