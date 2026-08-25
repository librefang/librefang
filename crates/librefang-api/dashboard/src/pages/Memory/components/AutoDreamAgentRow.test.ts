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
    const usage = {
      input_tokens: 50,
      cache_read_input_tokens: 40,
      cache_creation_input_tokens: 10,
    } as AutoDreamUsage;
    expect(autoDreamCacheStats(usage)).toEqual({
      hitPct: 40,
      totalInputTokens: 100,
    });
    expect(autoDreamCacheStats({ ...usage, input_tokens: 0 })).toBeNull();
    expect(autoDreamCacheStats(undefined)).toBeNull();
  });
});
