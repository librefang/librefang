import { describe, expect, it } from "vitest";
import { ApiError } from "./http/errors";
import { formatToastError } from "./errors";

function causalError(message: string, cause: Error): Error {
  return Object.assign(new Error(message), { cause });
}

describe("formatToastError", () => {
  it("does not disclose nested API causes in production text", () => {
    const err = new ApiError(500, "INTERNAL", "Request failed") as ApiError & {
      cause?: unknown;
    };
    err.cause = new Error("database at /srv/private/state.sqlite failed");
    expect(formatToastError(err, "Fallback", false)).toBe("[500] Request failed");
  });

  it("does not disclose nested generic causes in production text", () => {
    const err = causalError(
      "Operation failed",
      new Error("driver internals and private path"),
    );
    expect(formatToastError(err, "Fallback", false)).toBe("Operation failed");
  });

  it("uses one cause formatter for development diagnostics", () => {
    const cause = new Error("deep detail");
    expect(
      formatToastError(
        Object.assign(new ApiError(502, "UPSTREAM", "API failed"), { cause }),
        "Fallback",
        true,
      ),
    ).toBe("[502] API failed: deep detail");
    expect(
      formatToastError(causalError("Generic failed", cause), "Fallback", true),
    ).toBe("Generic failed: deep detail");
  });
});
