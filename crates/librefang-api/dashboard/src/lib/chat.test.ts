import { describe, expect, it } from "vitest";
import {
  applyForeignTerminalFrame,
  asText,
  extractAssistantHistoryParts,
  formatMeta,
  isTerminalFrameType,
  normalizeRole,
  terminalFrameOwner,
  normalizeToolOutput,
} from "./chat";
import type { ContentBlock } from "../api";
import type { TerminalFrame, TerminalRoutableMessage } from "./chat";

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

describe("terminalFrameOwner (#6390)", () => {
  it("treats a matching echoed id as the current turn", () => {
    expect(terminalFrameOwner("bot-1", "bot-1")).toBe("current");
  });

  it("routes a different echoed id to the foreign turn that owns it", () => {
    // The misbind race: turn A's late `response` arrives while turn B's listener is active — it must NOT be treated as B's terminal frame.
    expect(terminalFrameOwner("bot-A", "bot-B")).toBe("foreign");
  });

  it("keeps legacy behavior when the daemon echoes no id", () => {
    expect(terminalFrameOwner(undefined, "bot-1")).toBe("current");
    expect(terminalFrameOwner(null, "bot-1")).toBe("current");
    expect(terminalFrameOwner("", "bot-1")).toBe("current");
    expect(terminalFrameOwner(42, "bot-1")).toBe("current");
  });
});

describe("isTerminalFrameType (#6390)", () => {
  it("matches exactly the frames that end a turn's lifecycle", () => {
    expect(isTerminalFrameType("response")).toBe(true);
    expect(isTerminalFrameType("silent_complete")).toBe(true);
    expect(isTerminalFrameType("error")).toBe(true);
  });

  it("leaves streaming and lifecycle frames on the normal path", () => {
    for (const t of ["text_delta", "thinking_delta", "typing", "tool_start", "tool_end", "tool_result", "command_result", undefined]) {
      expect(isTerminalFrameType(t)).toBe(false);
    }
  });
});

describe("applyForeignTerminalFrame (#6390)", () => {
  // The exact misbind scenario: bubble A is a previous turn awaiting its
  // delayed terminal frame; bubble B is the current turn, mid-stream, whose
  // listener now receives A's frame. B must never be disturbed by A's frame.
  const bubbleA: TerminalRoutableMessage = { id: "bot-A", content: "partial A", isStreaming: true };
  const bubbleB: TerminalRoutableMessage = { id: "bot-B", content: "streaming B", isStreaming: true };

  it("patches the owning bubble with a foreign `response` and leaves the current turn's bubble untouched", () => {
    const frame: TerminalFrame = {
      type: "response",
      message_id: "bot-A",
      content: "full answer A",
      output_tokens: 20,
      input_tokens: 5,
      cost_usd: 0.002,
      memories_saved: ["m1"],
      memories_used: ["m2"],
      thinking: "reasoned A",
    };
    const next = applyForeignTerminalFrame([bubbleA, bubbleB], frame);
    expect(next[0].content).toBe("full answer A");
    expect(next[0].isStreaming).toBe(false);
    expect(next[0].tokens).toEqual({ output: 20, input: 5 });
    expect(next[0].cost_usd).toBe(0.002);
    expect(next[0].memories_saved).toEqual(["m1"]);
    expect(next[0].memories_used).toEqual(["m2"]);
    expect(next[0].thinking).toBe("reasoned A");
    // B is returned by reference — the newer turn's bubble and stream are never rewritten or re-rendered.
    expect(next[1]).toBe(bubbleB);
  });

  it("keeps the owning bubble's prior content when the foreign `response` carries none", () => {
    const next = applyForeignTerminalFrame([bubbleA], { type: "response", message_id: "bot-A", content: "" });
    expect(next[0].content).toBe("partial A");
    expect(next[0].isStreaming).toBe(false);
  });

  it("removes only the owning bubble on a foreign `silent_complete`", () => {
    const next = applyForeignTerminalFrame([bubbleA, bubbleB], { type: "silent_complete", message_id: "bot-A" });
    expect(next.map(m => m.id)).toEqual(["bot-B"]);
    expect(next[0]).toBe(bubbleB);
  });

  it("marks only the owning bubble on a foreign `error`, defaulting the message", () => {
    const withMsg = applyForeignTerminalFrame([bubbleA, bubbleB], { type: "error", message_id: "bot-A", content: "boom" });
    expect(withMsg[0].error).toBe("boom");
    expect(withMsg[0].isStreaming).toBe(false);
    expect(withMsg[1]).toBe(bubbleB);

    const noMsg = applyForeignTerminalFrame([bubbleA], { type: "error", message_id: "bot-A" });
    expect(noMsg[0].error).toBe("WebSocket error");
  });

  it("is a no-op when the frame's owner is not in the list", () => {
    const next = applyForeignTerminalFrame([bubbleA, bubbleB], { type: "response", message_id: "bot-missing", content: "x" });
    expect(next[0]).toBe(bubbleA);
    expect(next[1]).toBe(bubbleB);
  });
});
