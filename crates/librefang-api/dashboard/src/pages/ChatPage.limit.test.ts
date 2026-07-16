import { describe, expect, it } from "vitest";
import { isContextLimitError } from "./ChatPage";

// Pins real-world context-overflow phrases without conflating temporary rate limits or spending budgets with a session's context window.
describe("isContextLimitError", () => {
  it("returns false for empty / nullish input", () => {
    expect(isContextLimitError(undefined)).toBe(false);
    expect(isContextLimitError(null)).toBe(false);
    expect(isContextLimitError("")).toBe(false);
  });

  it("returns false for unrelated errors", () => {
    expect(isContextLimitError("connection refused")).toBe(false);
    expect(isContextLimitError("invalid api key")).toBe(false);
    expect(isContextLimitError("agent is suspended")).toBe(false);
  });

  it.each([
    "This model's maximum context length is 8192 tokens",
    "prompt is too long: 250000 tokens > 200000 maximum",
    "context_length_exceeded",
    "Input is too long for requested model.",
    "input_too_long",
    "string too long. Expected a string with maximum length 1048576",
    "Request exceeds the maximum allowed tokens",
    // Canonical phrase the kernel emits for a provider context-overflow; the banner must fire.
    "Context is full. Try /compact or /new.",
  ])("classifies context-overflow error: %s", (msg) => {
    expect(isContextLimitError(msg)).toBe(true);
  });

  it.each([
    "Usage budget reached for this window. This is a spending/usage cap, not a full context window — /compact will NOT help.",
    "Resource quota exceeded: Token limit would be exceeded: 195000 + 8192 reserved > 200000",
    "You have exceeded your quota. Please try again later.",
    "Rate limit reached for requests",
    "HTTP 429 Too Many Requests",
    "Token Limit reached",
    "Tool output is too long",
    "tool_output_too_long",
  ])("does not classify a non-context limit as context exhaustion: %s", (msg) => {
    expect(isContextLimitError(msg)).toBe(false);
  });

  it("treats structured error codes as authoritative", () => {
    expect(isContextLimitError("Rate limited", "context_length_exceeded")).toBe(true);
    expect(isContextLimitError("Context is full", "budget_exceeded")).toBe(false);
    expect(isContextLimitError("Context is full", "rate_limited")).toBe(false);
    expect(isContextLimitError("Context is full", "agent_not_found")).toBe(false);
    expect(isContextLimitError("Context is full", "session_agent_mismatch")).toBe(false);
    expect(isContextLimitError("Context is full", "message_delivery_failed")).toBe(false);
    expect(isContextLimitError("Context is full", "context_overflow")).toBe(false);
    expect(isContextLimitError("Tool output is too long", "format_error")).toBe(false);
  });

  it("keeps text fallback support for generic HTTP codes from older daemons", () => {
    expect(isContextLimitError("Context is full", "HTTP_500")).toBe(true);
  });

  it("is case-insensitive", () => {
    expect(isContextLimitError("CONTEXT WINDOW EXCEEDED")).toBe(true);
    expect(isContextLimitError("RESOURCE QUOTA EXCEEDED: TOKEN LIMIT WOULD BE EXCEEDED")).toBe(false);
  });
});
