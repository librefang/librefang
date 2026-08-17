import { describe, expect, it } from "vitest";
import { skillQueries } from "./skills";

describe("marketplace skill detail policy", () => {
  it("keeps every hub detail fresh for the same minute", () => {
    expect(skillQueries.clawhubSkill("skill").staleTime).toBe(60_000);
    expect(skillQueries.clawhubCnSkill("skill").staleTime).toBe(60_000);
    expect(skillQueries.skillhubSkill("skill").staleTime).toBe(60_000);
  });
});
