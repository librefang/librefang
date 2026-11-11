import { describe, expect, it } from "vitest";
import type { AutoDreamProgress, AutoDreamUsage } from "../../../api";
import {
  autoDreamCacheStats,
  autoDreamMoonClassName,
  autoDreamStatusVariant,
  lastAutoDreamTurn,
} from "./AutoDreamAgentRow";

describe("AutoDreamAgentRow helpers", () => {
  it("maps every dream status to an explicit badge variant", () => {
    expect(autoDreamStatusVariant("running")).toBe("info");
    expect(autoDreamStatusVariant("completed")).toBe("success");
    expect(autoDreamStatusVariant("aborted")).toBe("warning");
    expect(autoDreamStatusVariant("failed")).toBe("error");
  });

  it("animates the moon only for an enrolled running agent", () => {
    expect(autoDreamMoonClassName(true, true)).toContain("animate-pulse");
    expect(autoDreamMoonClassName(true, false)).not.toContain("animate-pulse");
    expect(autoDreamMoonClassName(false, true)).toContain("text-text-dim");
    expect(autoDreamMoonClassName(false, true)).not.toContain("animate-pulse");
  });

  it("returns the last turn and tolerates malformed missing turns", () => {
    const progress = { turns: [{ text: "first" }, { text: "last" }] } as AutoDreamProgress;
    expect(lastAutoDreamTurn(progress)?.text).toBe("last");
    expect(lastAutoDreamTurn({ turns: null } as unknown as AutoDreamProgress)).toBeUndefined();
    expect(lastAutoDreamTurn(null)).toBeUndefined();
  });

  it("derives cache stats outside JSX and suppresses empty usage", () => {
    // `input_tokens` is the whole prompt and already contains both buckets
    // (10 of the 100 tokens are cache writes, 40 are cache reads).
    const usage = {
      input_tokens: 100,
      cache_read_input_tokens: 40,
      cache_creation_input_tokens: 10,
    } as AutoDreamUsage;
    expect(autoDreamCacheStats(usage)).toEqual({
      hitPct: 40,
      totalInputTokens: 100,
    });
    expect(autoDreamCacheStats({ ...usage, input_tokens: 0 })).toBeNull();
    expect(autoDreamCacheStats(undefined)).toBeNull();
    // Buckets are subsets of the prompt, so their sum cannot exceed it.
    // `read == input` keeps this case invisible to a `read > input` check: only
    // the sum test rejects it, which is what keeps the guard from being relaxed
    // back to the read-only form.
    expect(
      autoDreamCacheStats({
        input_tokens: 100,
        cache_read_input_tokens: 100,
        cache_creation_input_tokens: 50,
      } as AutoDreamUsage),
    ).toBeNull();
    // The guard is strict on purpose: a sum equal to the prompt is a legitimate
    // full (or near-full) hit and must still render, so tightening `>` to `>=`
    // has to fail this file.
    expect(
      autoDreamCacheStats({
        input_tokens: 1000,
        cache_read_input_tokens: 990,
        cache_creation_input_tokens: 10,
      } as AutoDreamUsage),
    ).toEqual({ hitPct: 99, totalInputTokens: 1000 });
  });
});
