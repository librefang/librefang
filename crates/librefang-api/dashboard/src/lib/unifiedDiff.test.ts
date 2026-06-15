import { describe, expect, it } from "vitest";
import { unifiedLineDiff, hasChanges } from "./unifiedDiff";

describe("unifiedLineDiff", () => {
  it("returns all-context when texts are identical", () => {
    const diff = unifiedLineDiff("a\nb\nc\n", "a\nb\nc\n");
    expect(diff.map((l) => l.kind)).toEqual(["context", "context", "context"]);
    expect(hasChanges(diff)).toBe(false);
  });

  it("marks a changed middle line as remove + add", () => {
    const diff = unifiedLineDiff("a\nb\nc\n", "a\nB\nc\n");
    expect(diff).toEqual([
      { kind: "context", text: "a" },
      { kind: "remove", text: "b" },
      { kind: "add", text: "B" },
      { kind: "context", text: "c" },
    ]);
    expect(hasChanges(diff)).toBe(true);
  });

  it("handles a pure insertion", () => {
    const diff = unifiedLineDiff("a\nc\n", "a\nb\nc\n");
    expect(diff).toEqual([
      { kind: "context", text: "a" },
      { kind: "add", text: "b" },
      { kind: "context", text: "c" },
    ]);
  });

  it("handles a pure deletion", () => {
    const diff = unifiedLineDiff("a\nb\nc\n", "a\nc\n");
    expect(diff).toEqual([
      { kind: "context", text: "a" },
      { kind: "remove", text: "b" },
      { kind: "context", text: "c" },
    ]);
  });

  it("treats an empty old body as all-add (new skill)", () => {
    const diff = unifiedLineDiff("", "x\ny\n");
    expect(diff).toEqual([
      { kind: "add", text: "x" },
      { kind: "add", text: "y" },
    ]);
  });

  it("does not emit a spurious trailing blank row for terminal newlines", () => {
    const diff = unifiedLineDiff("a\n", "a\nb\n");
    expect(diff).toEqual([
      { kind: "context", text: "a" },
      { kind: "add", text: "b" },
    ]);
  });
});
