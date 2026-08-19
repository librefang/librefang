import { describe, expect, it } from "vitest";
import i18n, { i18nReady, normalizeDetectedLanguage } from "./i18n";

describe("dashboard i18n initialization", () => {
  it.each([
    ["en-US", "en"],
    ["zh-CN", "zh"],
    ["ko_KR", "ko"],
    ["UK-ua", "uk"],
  ])("normalizes detected locale %s to %s", (detected, expected) => {
    expect(normalizeDetectedLanguage(detected)).toBe(expected);
  });

  it("exposes a handled readiness promise with language-only loading", async () => {
    await expect(i18nReady).resolves.toBeDefined();
    expect(i18n.options.load).toBe("languageOnly");
    expect(i18n.options.supportedLngs).toEqual(
      expect.arrayContaining(["en", "zh", "uk", "ko", "pl"]),
    );
  });
});
