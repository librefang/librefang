import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import en from "../locales/en.json";
import { CHANGE_SOURCES, MANIFEST_CHANGE_SOURCES } from "../api";
import { changeSourceLabel, manifestChangeSourceLabel } from "./changeSource";

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
    expect(changeSourceLabel(translate, "toml")).toBe("Raw TOML edit");
    expect(changeSourceLabel(translate, "registry-restore")).toBe(
      "Restored from registry",
    );
    expect(changeSourceLabel(translate, "pre-registry-restore")).toBe(
      "Pre-restore snapshot",
    );
    expect(changeSourceLabel(translate, "agent-tool")).toBe("Agent tool");
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

describe("manifestChangeSourceLabel", () => {
  // The closed inventory of `manifest_versions.change_source` values the kernel can write, each
  // pinned to its exact label: a source added to the store without a label fails here, and so does
  // a label someone renames — the History tab and the destructive restore dialog both read from it.
  it.each([
    ["api", "API edit"],
    ["model", "Model change"],
    ["skills", "Skills change"],
    ["workspaces", "Workspaces change"],
    ["channels", "Channels change"],
    ["routing", "Routing change"],
    ["schedule", "Schedule change"],
    ["tool-filters", "Tool filters change"],
    ["mcp-servers", "MCP servers change"],
    ["provisioning", "Provisioned"],
    ["restore", "Restored"],
    ["suspend", "Suspended"],
    ["resume", "Resumed"],
    ["suspend-persist-failed", "Suspend write failed"],
    ["resume-persist-failed", "Resume write failed"],
    ["unknown", "Unknown source"],
  ] as const)("labels the manifest source %s as %s", (source, label) => {
    expect(manifestChangeSourceLabel(translate, source)).toBe(label);
  });

  it.each(MANIFEST_CHANGE_SOURCES)(
    "gives the known manifest source %s a catalog label",
    (source) => {
      const label = manifestChangeSourceLabel(translate, source);
      expect(label).not.toBe(`agents.detail.manifest_change_source.${source}`);
      expect(label).not.toBe(source);
      expect(label.trim()).not.toBe("");
    },
  );

  // A producer the dashboard has not been taught, or a row from an older database, must still say where it came from.
  it("returns an unmapped manifest source verbatim", () => {
    expect(manifestChangeSourceLabel(translate, "some_future_source")).toBe(
      "some_future_source",
    );
  });

  // Same guard as the agent-type vocabulary: a dotted token checked against the map first cannot become a locale path.
  it("does not resolve an unmapped dotted manifest source as a locale key", () => {
    expect(manifestChangeSourceLabel(translate, "agents.detail.manifest_history")).toBe(
      "agents.detail.manifest_history",
    );
  });
});
