import { describe, expect, it } from "vitest";
import { prettifyToolName, truncate } from "./string";

describe("string helpers", () => {
  it.each(["---", "_._", "..."])(
    "uses the tool fallback for separator-only name %s",
    (name) => expect(prettifyToolName(name)).toBe("tool"),
  );

  it("preserves readable tool-name formatting", () => {
    expect(prettifyToolName("MCP_call.run")).toBe("MCP Call Run");
  });

  it("returns an empty truncation for non-positive limits", () => {
    expect(truncate("abc", 0)).toBe("");
    expect(truncate("abc", -2)).toBe("");
  });
});
