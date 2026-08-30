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
    expect(hub?.cli("private/tool")).toBe(
      `CLAWHUB_REGISTRY="$SKILLHUB_REGISTRY_URL" clawhub install 'private/tool'`,
    );
  });

  it("uses a valid configured self-hosted registry consistently", async () => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", "https://skills.example.com/registry/");
    const { getSkillHub } = await import("./skillHubs");
    const hub = getSkillHub("skillhub");

    expect(hub?.domain).toBe("skills.example.com");
    expect(hub?.cli("private/tool")).toBe(
      `CLAWHUB_REGISTRY='https://skills.example.com/registry' clawhub install 'private/tool'`,
    );
  });

  it.each([
    "javascript:alert(1)",
    "https://user:secret@skills.example.com/registry",
    "https://skills.example.com/registry?token=public",
    "https://skills.example.com/registry#fragment",
  ])("rejects unsafe registry configuration %s", async (registryUrl) => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", registryUrl);
    const { getSkillHub } = await import("./skillHubs");

    expect(getSkillHub("skillhub")?.domain).toBe("deployment configured");
  });

  it("normalizes every trailing registry slash", async () => {
    vi.stubEnv(
      "VITE_SKILLHUB_REGISTRY_URL",
      "https://skills.example.com/registry///",
    );
    const { getSkillHub } = await import("./skillHubs");

    expect(getSkillHub("skillhub")?.cli("private/tool")).toBe(
      `CLAWHUB_REGISTRY='https://skills.example.com/registry' clawhub install 'private/tool'`,
    );
  });

  it("shell-quotes registry URLs and remote slugs", async () => {
    vi.stubEnv(
      "VITE_SKILLHUB_REGISTRY_URL",
      "https://skills.example.com/$HOME",
    );
    const { getSkillHub } = await import("./skillHubs");

    expect(getSkillHub("skillhub")?.cli("tool'; touch /tmp/pwn; echo '")).toBe(
      `CLAWHUB_REGISTRY='https://skills.example.com/$HOME' clawhub install 'tool'"'"'; touch /tmp/pwn; echo '"'"''`,
    );
    expect(getSkillHub("fanghub")?.cli("$(touch /tmp/pwn)")).toBe(
      `librefang skill install '$(touch /tmp/pwn)'`,
    );
  });
});

describe("skillHubUrl", () => {
  it("points ClawHub and its CN mirror at their public skill pages", async () => {
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("clawhub", "git-helper")).toBe(
      "https://clawhub.ai/skills/git-helper",
    );
    expect(skillHubUrl("clawhub-cn", "git-helper")).toBe(
      "https://mirror-cn.clawhub.com/skills/git-helper",
    );
  });

  it("offers no link for FangHub, which has no public skill page", async () => {
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("fanghub", "pdf-tools")).toBeNull();
  });

  it("offers no link for an unconfigured self-hosted SkillHub", async () => {
    vi.stubEnv("VITE_SKILLHUB_REGISTRY_URL", "");
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("skillhub", "pdf-tools")).toBeNull();
  });

  it("derives the SkillHub page from the configured registry origin", async () => {
    vi.stubEnv(
      "VITE_SKILLHUB_REGISTRY_URL",
      "https://skills.example.com/api/v1",
    );
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("skillhub", "pdf-tools")).toBe(
      "https://skills.example.com/skills/pdf-tools",
    );
  });

  it("returns null for unknown hubs and blank slugs", async () => {
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("unknown-runtime-hub", "git-helper")).toBeNull();
    expect(skillHubUrl(undefined, "git-helper")).toBeNull();
    expect(skillHubUrl("clawhub", undefined)).toBeNull();
    expect(skillHubUrl("clawhub", "   ")).toBeNull();
  });

  it("escapes the slug so it cannot rewrite the path", async () => {
    const { skillHubUrl } = await import("./skillHubs");

    expect(skillHubUrl("clawhub", "../../evil")).toBe(
      "https://clawhub.ai/skills/..%2F..%2Fevil",
    );
  });

  it("never yields a non-https URL for any hub", async () => {
    const { SKILL_HUBS } = await import("./skillHubs");

    for (const hub of SKILL_HUBS) {
      const url = hub.skillUrl("sample");
      if (url !== null) expect(url.startsWith("https://")).toBe(true);
    }
  });
});
