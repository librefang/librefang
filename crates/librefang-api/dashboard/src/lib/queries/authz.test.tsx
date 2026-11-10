import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApiError, getEffectivePermissions } from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { authzQueries, canListAllAgents, useEffectivePermissions } from "./authz";

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

describe("canListAllAgents (#8596)", () => {
  it("admits only the roles whose agent listing is unscoped", () => {
    // The daemon injects an owner filter for every role below Admin
    // (`routes/agents/lifecycle.rs`), so a User's list is a view of its own
    // agents, not the registry — and `undefined` is whoami still in flight,
    // which must not read as Admin.
    expect(canListAllAgents("admin")).toBe(true);
    expect(canListAllAgents("owner")).toBe(true);
    expect(canListAllAgents("user")).toBe(false);
    expect(canListAllAgents("viewer")).toBe(false);
    expect(canListAllAgents(undefined)).toBe(false);
    expect(canListAllAgents("")).toBe(false);
  });
});
