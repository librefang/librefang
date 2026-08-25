import { describe, expect, it } from "vitest";

import type { ModelItem } from "../api";
import { filterVisible, modelKey } from "./hiddenModels";

describe("hidden model keys", () => {
  it("keeps provider/id partitions distinct when either contains colons", () => {
    const providerColon = modelKey({ provider: "a:b", id: "c" });
    const idColon = modelKey({ provider: "a", id: "b:c" });

    expect(providerColon).not.toBe(idColon);
  });

  it("filters only the exact hidden model", () => {
    const models = [
      { provider: "a:b", id: "c" },
      { provider: "a", id: "b:c" },
    ] as ModelItem[];

    expect(filterVisible(models, new Set([modelKey(models[0])]))).toEqual([models[1]]);
  });
});
