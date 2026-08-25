import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  AVAILABLE_PROVIDER_STATUSES,
  getStatusVariant,
  isProviderAvailable,
} from "./status";

function snakeCase(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

describe("provider status contracts", () => {
  it("normalizes availability with the same case policy as badge variants", () => {
    expect(isProviderAvailable("Validated_Key")).toBe(true);
    expect(isProviderAvailable("CONFIGURED_CLI")).toBe(true);
    expect(getStatusVariant("RUNNING")).toBe("success");
  });

  it("mirrors the Rust AuthStatus::is_available variant set", () => {
    const source = readFileSync(
      resolve(process.cwd(), "../../librefang-types/src/model_catalog.rs"),
      "utf8",
    );
    const body = source.match(
      /pub fn is_available\(self\) -> bool \{([\s\S]*?)\n[ ]{4}\}/,
    )?.[1];
    expect(body, "AuthStatus::is_available body must be discoverable").toBeTruthy();
    const rustStatuses = [...(body ?? "").matchAll(/AuthStatus::([A-Za-z]+)/g)]
      .map((match) => snakeCase(match[1]))
      .sort();

    expect([...AVAILABLE_PROVIDER_STATUSES].sort()).toEqual(rustStatuses);
  });
});
