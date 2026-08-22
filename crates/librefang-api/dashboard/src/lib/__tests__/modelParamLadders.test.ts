import { describe, expect, it } from "vitest";
import {
  CONTEXT_WINDOW_LADDER,
  MAX_OUTPUT_TOKENS_LADDER,
  formatTokens,
  isOnLadder,
  ladderUpTo,
} from "../modelParamLadders";

describe("modelParamLadders", () => {
  it("keeps both ladders ascending", () => {
    for (const ladder of [CONTEXT_WINDOW_LADDER, MAX_OUTPUT_TOKENS_LADDER]) {
      const sorted = [...ladder].sort((a, b) => a - b);
      expect([...ladder]).toEqual(sorted);
      expect(new Set(ladder).size).toBe(ladder.length);
    }
  });

  // The two ladders are not interchangeable. Output tokens are what a model
  // generates; context tokens are what it reads. Gemini's 1M/2M are context
  // figures, and putting them on the output ladder would advertise a setting
  // no provider will honour — worse than the slider this replaced, because it
  // asserts the value is valid.
  it("stops the output ladder well below the context ladder", () => {
    const topContext = CONTEXT_WINDOW_LADDER[CONTEXT_WINDOW_LADDER.length - 1];
    const topOutput = MAX_OUTPUT_TOKENS_LADDER[MAX_OUTPUT_TOKENS_LADDER.length - 1];
    expect(topContext).toBe(2_097_152);
    expect(topOutput).toBe(131_072);
    expect(topOutput).toBeLessThan(topContext);
  });

  it("mirrors the Rust ladders the TUI uses", () => {
    // Kept in lockstep with CONTEXT_WINDOW_LADDER / MAX_OUTPUT_TOKENS_LADDER in
    // crates/librefang-types/src/inference_params.rs. If this fails, one side
    // was changed alone and the two editors now disagree.
    expect([...CONTEXT_WINDOW_LADDER]).toEqual([
      8_192, 32_768, 131_072, 262_144, 524_288, 1_048_576, 2_097_152,
    ]);
    expect([...MAX_OUTPUT_TOKENS_LADDER]).toEqual([
      1_024, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072,
    ]);
  });

  it("formats token counts the way operators read them", () => {
    expect(formatTokens(8_192)).toBe("8K");
    expect(formatTokens(131_072)).toBe("128K");
    expect(formatTokens(1_048_576)).toBe("1M");
    expect(formatTokens(2_097_152)).toBe("2M");
    expect(formatTokens(50_000)).toBe("50000");
  });

  it("trims the ladder to a declared cap and keeps the cap selectable", () => {
    expect(ladderUpTo(MAX_OUTPUT_TOKENS_LADDER, 16_384)).toEqual([1_024, 4_096, 8_192, 16_384]);
    // A cap between two rungs is appended rather than rounded away, which is
    // the only way to offer a model whose real maximum is 20000.
    expect(ladderUpTo(MAX_OUTPUT_TOKENS_LADDER, 20_000)).toEqual([
      1_024, 4_096, 8_192, 16_384, 20_000,
    ]);
  });

  // An unknown limit is not a ceiling. Capping against a discovery placeholder
  // would hide rungs the endpoint may well support (#7780).
  it("leaves the ladder whole when no cap was sourced", () => {
    expect(ladderUpTo(MAX_OUTPUT_TOKENS_LADDER, undefined)).toEqual([...MAX_OUTPUT_TOKENS_LADDER]);
    expect(ladderUpTo(MAX_OUTPUT_TOKENS_LADDER, 0)).toEqual([...MAX_OUTPUT_TOKENS_LADDER]);
  });

  it("treats an off-ladder value as custom", () => {
    expect(isOnLadder(MAX_OUTPUT_TOKENS_LADDER, 8_192)).toBe(true);
    expect(isOnLadder(MAX_OUTPUT_TOKENS_LADDER, 50_000)).toBe(false);
    expect(isOnLadder(MAX_OUTPUT_TOKENS_LADDER, null)).toBe(false);
  });
});
