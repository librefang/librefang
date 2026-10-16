import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import en from "../locales/en.json";
import { CHANGE_SOURCES, SKILL_CHANGE_SOURCES } from "../api";
import { changeSourceLabel, skillChangeSourceLabel } from "./changeSource";

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

describe("skillChangeSourceLabel", () => {
  // The skill store's vocabulary is distinct from the template and manifest ones (#8021), so every token is pinned here: a row in the history list and the destructive restore dialog both read from this map.
  it.each([
    ["create", "Created"],
    ["update", "Updated"],
    ["evolve", "Evolved"],
    ["patch", "Patched"],
    ["rollback", "Rolled back"],
    ["install", "Installed"],
    ["uninstall", "Uninstalled"],
    ["delete", "Deleted"],
    ["file-write", "File written"],
    ["file-remove", "File removed"],
    ["restore", "Restored"],
    ["unknown", "Unknown source"],
  ] as const)("labels the skill source %s as %s", (source, label) => {
    expect(skillChangeSourceLabel(translate, source)).toBe(label);
  });

  it.each(SKILL_CHANGE_SOURCES)(
    "gives the known skill source %s a catalog label",
    (source) => {
      const label = skillChangeSourceLabel(translate, source);
      expect(label).not.toBe(`skills.change_source.${source}`);
      expect(label).not.toBe(source);
      expect(label.trim()).not.toBe("");
    },
  );

  // A producer the dashboard has not been taught, or a row from an older database, must still say where it came from.
  it("returns an unmapped skill source verbatim", () => {
    expect(skillChangeSourceLabel(translate, "some_future_source")).toBe(
      "some_future_source",
    );
  });

  // Same guard as the other vocabularies: a dotted token checked against the map first cannot become a locale path.
  it("does not resolve an unmapped dotted skill source as a locale key", () => {
    expect(skillChangeSourceLabel(translate, "skills.evo_history")).toBe(
      "skills.evo_history",
    );
  });
});
