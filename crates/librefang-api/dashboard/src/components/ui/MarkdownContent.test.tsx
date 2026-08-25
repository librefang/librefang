import { describe, expect, it } from "vitest";
import { urlTransform } from "./MarkdownContent";

describe("MarkdownContent urlTransform", () => {
  it.each([
    "obsidian://open?vault=Notes&file=Projects%2FLibreFang.md",
    "obsidian-advanced-uri://open?vault=Notes&filepath=Daily%20Notes%2Ftoday.md#Tasks",
  ])("allows a valid Obsidian deep link: %s", (url) => {
    expect(urlTransform(url)).toBe(url);
  });

  it.each([
    'obsidian://open?vault=Notes" onclick="alert(1)',
    "obsidian://open?vault=Notes and file=secret",
    "obsidian://open?vault=Notes\\file=secret",
    "obsidian://open?vault=Notes\u0000file=secret",
  ])("rejects invalid characters in an Obsidian deep link", (url) => {
    expect(urlTransform(url)).toBe("");
  });

  it("retains the default URL policy for other schemes", () => {
    expect(urlTransform("https://example.com/docs")).toBe("https://example.com/docs");
    expect(urlTransform("javascript:alert(1)")).toBe("");
  });
});
