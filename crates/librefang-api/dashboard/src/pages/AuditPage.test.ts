import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "../lib/http/errors";
import {
  auditFiltersToParams,
  filtersFromRouteSearch,
  formatAuditExportError,
  isCustomAuditChannel,
  routeSearchIdentity,
  scheduleObjectUrlRevoke,
} from "./AuditPage";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("AuditPage state helpers", () => {
  it("serializes only populated filter values", () => {
    expect(
      auditFiltersToParams({
        user: "owner",
        action: "",
        channel: undefined,
        limit: 200,
      }).toString(),
    ).toBe("user=owner&limit=200");
  });

  it("keeps explicit custom-channel mode open for a blank value", () => {
    expect(isCustomAuditChannel(undefined, ["", "api"], true)).toBe(true);
    expect(isCustomAuditChannel("webhook-a", ["", "api"], false)).toBe(true);
    expect(isCustomAuditChannel("api", ["", "api"], false)).toBe(false);
  });

  it("derives filters and identity from every route update", () => {
    expect(filtersFromRouteSearch({ user: "one" })).toEqual({
      user: "one",
      action: undefined,
      agent: undefined,
      channel: undefined,
      from: undefined,
      to: undefined,
      limit: 200,
    });
    expect(routeSearchIdentity({ user: "one", seq: 1 })).not.toBe(
      routeSearchIdentity({ user: "two", seq: 2 }),
    );
  });

  it("formats structured and ordinary export failures", () => {
    expect(formatAuditExportError(new ApiError(403, "FORBIDDEN", "Denied"))).toBe(
      "403: Denied",
    );
    expect(formatAuditExportError(new Error("Network failed"))).toBe(
      "Network failed",
    );
  });

  it("revokes the previous object URL before replacing its timer", () => {
    vi.useFakeTimers();
    const revokeObjectURL = vi.fn();
    const originalDescriptor = Object.getOwnPropertyDescriptor(
      URL,
      "revokeObjectURL",
    );
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });

    try {
      scheduleObjectUrlRevoke("blob:first");
      scheduleObjectUrlRevoke("blob:second");

      expect(revokeObjectURL).toHaveBeenCalledWith("blob:first");
      vi.advanceTimersByTime(1000);
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:second");
    } finally {
      if (originalDescriptor) {
        Object.defineProperty(URL, "revokeObjectURL", originalDescriptor);
      } else {
        Reflect.deleteProperty(URL, "revokeObjectURL");
      }
    }
  });
});
