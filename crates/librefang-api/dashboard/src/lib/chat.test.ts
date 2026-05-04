import { describe, expect, it } from "vitest";
import {
  asText,
  extractAssistantHistoryParts,
  formatMeta,
  normalizeRole,
  normalizeToolOutput,
} from "./chat";
import type { ContentBlock } from "../api";

describe("chat utilities", () => {
  it("normalizes API message roles", () => {
    expect(normalizeRole("User")).toBe("user");
    expect(normalizeRole("System")).toBe("system");
    expect(normalizeRole("Assistant")).toBe("assistant");
  });

  it("converts unknown values to text", () => {
    expect(asText("hello")).toBe("hello");
    expect(asText({ ok: true })).toContain('"ok": true');
  });

  it("formats usage metadata", () => {
    expect(
      formatMeta({
        input_tokens: 12,
        output_tokens: 34,
        iterations: 2,
        cost_usd: 0.00123
      })
    ).toBe("12 in / 34 out | 2 iter | $0.0012");
  });

  it("normalizes tool output events for persistent display", () => {
    const output = normalizeToolOutput({
      tool: "display_device_id",
      result: "Device ID: abc-123",
      is_error: false,
    });

    expect(output).not.toBeNull();
    expect(output?.tool).toBe("display_device_id");
    expect(output?.content).toContain("Device ID");
    expect(output?.isError).toBe(false);
  });

  it("ignores malformed tool output events", () => {
    expect(normalizeToolOutput({ result: "hello" })).toBeNull();
    expect(normalizeToolOutput({})).toBeNull();
    expect(normalizeToolOutput({ tool: "" })).toBeNull();
  });

  it("handles error tool outputs", () => {
    const output = normalizeToolOutput({
      tool: "shell_exec",
      is_error: true,
    });
    expect(output?.isError).toBe(true);
    expect(output?.content).toBe("Tool failed without a preview.");
  });
});

describe("extractAssistantHistoryParts", () => {
  it("returns plain string content unchanged as text, with empty thinking", () => {
    expect(extractAssistantHistoryParts("hello world")).toEqual({
      text: "hello world",
      thinking: "",
    });
  });

  it("returns empty parts for null/undefined content", () => {
    expect(extractAssistantHistoryParts(null)).toEqual({ text: "", thinking: "" });
    expect(extractAssistantHistoryParts(undefined)).toEqual({ text: "", thinking: "" });
  });

  it("extracts text blocks and joins them with a blank-line paragraph break", () => {
    // Adjacent text blocks separated by a single `\n` would render as
    // one paragraph in react-markdown — we use `\n\n` so distinct
    // chunks don't fuse visually.
    const blocks: ContentBlock[] = [
      { type: "text", text: "first" },
      { type: "text", text: "second" },
    ];
    expect(extractAssistantHistoryParts(blocks)).toEqual({
      text: "first\n\nsecond",
      thinking: "",
    });
  });

  it("extracts thinking blocks and joins them with double newlines", () => {
    const blocks: ContentBlock[] = [
      { type: "thinking", thinking: "step 1" },
      { type: "thinking", thinking: "step 2" },
    ];
    expect(extractAssistantHistoryParts(blocks)).toEqual({
      text: "",
      thinking: "step 1\n\nstep 2",
    });
  });

  it("handles mixed thinking + text + tool_use, ignoring tool blocks", () => {
    const blocks: ContentBlock[] = [
      { type: "thinking", thinking: "let me think" },
      { type: "tool_use", id: "t1", name: "shell", input: { cmd: "ls" } },
      { type: "text", text: "here is the result" },
      { type: "thinking", thinking: "more thinking" },
      { type: "text", text: "final answer" },
    ];
    expect(extractAssistantHistoryParts(blocks)).toEqual({
      text: "here is the result\n\nfinal answer",
      thinking: "let me think\n\nmore thinking",
    });
  });

  it("collapses interleaved thinking/text into independent buckets (matches live-streaming)", () => {
    // `[thinking A, text X, thinking B, text Y]` → text and thinking
    // each accumulate in order but the cross-bucket interleave is
    // intentionally lost (see JSDoc on extractAssistantHistoryParts).
    // Locking this here so a future "preserve interleave" change comes
    // with a deliberate review of the live-streaming parity.
    const blocks: ContentBlock[] = [
      { type: "thinking", thinking: "A" },
      { type: "text", text: "X" },
      { type: "thinking", thinking: "B" },
      { type: "text", text: "Y" },
    ];
    expect(extractAssistantHistoryParts(blocks)).toEqual({
      text: "X\n\nY",
      thinking: "A\n\nB",
    });
  });

  it("silently skips redacted_thinking and unknown block types (forward-compat)", () => {
    // redacted_thinking handling is deferred — see follow-up.
    // Treat as unknown so plaintext thinking still surfaces.
    const blocks = [
      { type: "redacted_thinking", data: "encrypted" },
      { type: "thinking", thinking: "visible reasoning" },
      { type: "future_block", payload: 42 },
    ] as unknown as ContentBlock[];
    expect(extractAssistantHistoryParts(blocks)).toEqual({
      text: "",
      thinking: "visible reasoning",
    });
  });

  it("returns empty parts for an empty block array", () => {
    expect(extractAssistantHistoryParts([])).toEqual({ text: "", thinking: "" });
  });

  it("falls back to String(value) for non-string, non-array, non-null content", () => {
    // Defensive: server should never send a number, but if it does we
    // should not throw and we should not corrupt the chat transcript.
    expect(extractAssistantHistoryParts(42 as unknown as string)).toEqual({
      text: "42",
      thinking: "",
    });
  });

  it("falls back to String(value) for an object that is not wrapped in an array", () => {
    // Defensive: a single block sent unwrapped is a server contract
    // violation. The fallback `String(value)` produces the unhelpful
    // `"[object Object]"`, but the explicit assertion locks the
    // behavior so a future "auto-wrap into a single-block array"
    // refactor doesn't change this code path silently. The transcript
    // remains uncorrupted by structured blocks bleeding into the text
    // path.
    const stray = { type: "text", text: "x" };
    expect(extractAssistantHistoryParts(stray as unknown as string)).toEqual({
      text: "[object Object]",
      thinking: "",
    });
  });
});
