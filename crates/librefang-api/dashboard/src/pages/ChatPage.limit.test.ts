import { describe, expect, it } from "vitest";
import { isContextLimitError } from "./ChatPage";

// Pins real-world provider error phrases so a refactor can't silently break context-limit detection.
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
    "string too long. Expected a string with maximum length 1048576",
    "Request exceeds the maximum allowed tokens",
    "You have exceeded your quota. Please try again later.",
    "Rate limit reached for requests",
    "HTTP 429 Too Many Requests",
  ])("classifies provider limit error: %s", (msg) => {
    expect(isContextLimitError(msg)).toBe(true);
  });

  it("is case-insensitive", () => {
    expect(isContextLimitError("CONTEXT WINDOW EXCEEDED")).toBe(true);
    expect(isContextLimitError("Token Limit reached")).toBe(true);
  });
});
