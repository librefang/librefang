import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import en from "../locales/en.json";
import { CHANGE_SOURCES } from "../api";
import { changeSourceLabel } from "./changeSource";

// Resolve keys against the real English catalog so a producer added to `CHANGE_SOURCES` without a label fails here instead of rendering its key.
const translate = ((key: string) => {
  const value = key
    .split(".")
    .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en);
  return typeof value === "string" ? value : key;
}) as unknown as TFunction;

describe("changeSourceLabel", () => {
  it.each(CHANGE_SOURCES)("gives the known source %s an English label", (source) => {
    const label = changeSourceLabel(translate, source);
    expect(label).not.toBe(`agentTypes.change_source.${source}`);
    expect(label).not.toBe(source);
    expect(label.trim()).not.toBe("");
  });

  it("labels the values the server writes today", () => {
    expect(changeSourceLabel(translate, "create")).toBe("Created");
    expect(changeSourceLabel(translate, "dashboard")).toBe("Dashboard edit");
    expect(changeSourceLabel(translate, "restore")).toBe("Restored");
    expect(changeSourceLabel(translate, "unknown")).toBe("Unknown source");
  });

  // A new producer, or a row written by an older daemon, still has to say where it came from.
  it("returns an unmapped source verbatim", () => {
    expect(changeSourceLabel(translate, "some_future_source")).toBe("some_future_source");
  });

  // The known-value check is what keeps a dotted token from being read as a nested locale path.
  it("does not resolve an unmapped dotted source as a locale key", () => {
    expect(changeSourceLabel(translate, "agentTypes.title")).toBe("agentTypes.title");
  });
});
