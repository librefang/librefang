import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApiError, getEffectivePermissions } from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { authzQueries, useEffectivePermissions } from "./authz";

vi.mock("../http/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../http/client")>();
  return {
    ...original,
    getEffectivePermissions: vi.fn(),
  };
});

describe("effective permission query", () => {
  it("cannot be enabled without a user name", () => {
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useEffectivePermissions("", { enabled: true }), { wrapper });

    expect(getEffectivePermissions).not.toHaveBeenCalled();
  });

  it("does not retry deterministic authorization failures", () => {
    const retry = authzQueries.effective("alice").retry;
    const forbidden = new ApiError(403, "FORBIDDEN", "forbidden");
    const missing = new ApiError(404, "NOT_FOUND", "missing");

    expect(typeof retry).toBe("function");
    if (typeof retry !== "function") return;
    expect(retry(0, forbidden)).toBe(false);
    expect(retry(0, missing)).toBe(false);
  });

  it("retries transient failures up to three times", () => {
    const retry = authzQueries.effective("alice").retry;

    expect(typeof retry).toBe("function");
    if (typeof retry !== "function") return;
    expect(retry(0, new Error("offline"))).toBe(true);
    expect(retry(2, new Error("offline"))).toBe(true);
    expect(retry(3, new Error("offline"))).toBe(false);
  });
});
