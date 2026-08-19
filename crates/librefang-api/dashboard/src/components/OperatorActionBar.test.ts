import { describe, expect, it } from "vitest";
import { describeAction } from "./OperatorActionBar";

const translate = (
  _key: string,
  fallback: string,
  _options?: Record<string, unknown>,
) => fallback;

describe("describeAction", () => {
  it("rejects unknown string verbs instead of treating them as objects", () => {
    expect(describeAction("future_action", translate)).toBeNull();
  });

  it("rejects malformed object descriptors", () => {
    expect(describeAction({}, translate)).toBeNull();
    expect(describeAction({ provide_input: null }, translate)).toBeNull();
    expect(describeAction({ provide_input: {} }, translate)).toBeNull();
  });

  it("preserves valid provide-input descriptors", () => {
    expect(
      describeAction({ provide_input: { field: "ticket_id" } }, translate),
    ).toMatchObject({
      verb: "provide_input",
      field: "ticket_id",
      needsPayload: true,
    });
  });
});
