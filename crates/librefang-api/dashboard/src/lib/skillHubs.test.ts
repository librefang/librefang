import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllEnvs();
  vi.resetModules();
});

describe("skill hub configuration", () => {
  it("has a complete, unique index and returns undefined for unknown runtime ids", async () => {
    const { getSkillHub, SKILL_HUBS } = await import("./skillHubs");

    expect(SKILL_HUBS.map((hub) => hub.id)).toEqual([
      "fanghub",
      "skillhub",
      "clawhub",
      "clawhub-cn",
    ]);
    expect(new Set(SKILL_HUBS.map((hub) => hub.id)).size).toBe(SKILL_HUBS.length);
    for (const hub of SKILL_HUBS) {
      expect(getSkillHub(hub.id)).toBe(hub);
    }
    expect(getSkillHub("unknown-runtime-hub")).toBeUndefined();
  });

  it("uses a deployment shell variable when no self-hosted URL is configured", async () => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", "");
    const { getSkillHub } = await import("./skillHubs");
    const hub = getSkillHub("skillhub");

    expect(hub?.domain).toBe("deployment configured");
    expect(hub?.cli("private/tool")).toContain(
      'CLAWHUB_REGISTRY="$SKILLHUB_REGISTRY_URL"',
    );
  });

  it("uses a valid configured self-hosted registry consistently", async () => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", "https://skills.example.com/registry/");
    const { getSkillHub } = await import("./skillHubs");
    const hub = getSkillHub("skillhub");

    expect(hub?.domain).toBe("skills.example.com");
    expect(hub?.cli("private/tool")).toContain(
      'CLAWHUB_REGISTRY="https://skills.example.com/registry"',
    );
  });

  it("rejects non-HTTP registry configuration", async () => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", "javascript:alert(1)");
    const { getSkillHub } = await import("./skillHubs");
    expect(getSkillHub("skillhub")?.domain).toBe("deployment configured");
  });
});
