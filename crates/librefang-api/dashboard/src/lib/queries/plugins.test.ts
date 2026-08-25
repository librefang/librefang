import { describe, expect, it } from "vitest";
import { pluginQueries } from "./plugins";

describe("plugin query policy", () => {
  it("keeps each foreground poll aligned with its freshness window", () => {
    const plugins = pluginQueries.list();
    const registries = pluginQueries.registries();

    expect(plugins.staleTime).toBe(60_000);
    expect(plugins.refetchInterval).toBe(plugins.staleTime);
    expect(registries.staleTime).toBe(5 * 60_000);
    expect(registries.refetchInterval).toBe(registries.staleTime);
    expect(plugins.refetchIntervalInBackground).toBe(false);
    expect(registries.refetchIntervalInBackground).toBe(false);
  });
});
