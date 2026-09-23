import { describe, expect, it } from "vitest";
import { unknownKeysWarning } from "./agentTypes";
import type { AgentTypeDetail } from "../../api";

// #8028: the server reports `unknown_keys` in the save response when the
// submitted TOML carried a key `AgentManifest` doesn't recognize and
// therefore dropped, but nothing in the only client that calls this
// endpoint read the field — the dashboard showed "saved" and the operator
// never learned a key silently vanished from their file.
describe("unknownKeysWarning", () => {
  const base: AgentTypeDetail = {
    name: "researcher",
    source: "agent-type",
    editable: true,
    spec: {},
    manifest_toml: "",
  };

  it("returns null when the save reported no unknown keys", () => {
    expect(unknownKeysWarning(base)).toBeNull();
    expect(unknownKeysWarning({ ...base, unknown_keys: [] })).toBeNull();
  });

  it("joins the dropped keys into a single readable string", () => {
    expect(unknownKeysWarning({ ...base, unknown_keys: ["future_field"] })).toBe("future_field");
    expect(
      unknownKeysWarning({ ...base, unknown_keys: ["sesion_mode", "future_field"] }),
    ).toBe("sesion_mode, future_field");
  });
});
