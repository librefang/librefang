import { describe, expect, it, vi } from "vitest";
import { ApiError } from "./errors";

describe("ApiError", () => {
  it("preserves its prototype chain", () => {
    const error = new ApiError(400, "BAD_REQUEST", "Bad request");

    expect(error).toBeInstanceOf(Error);
    expect(error).toBeInstanceOf(ApiError);
  });

  it("normalizes response body read failures", async () => {
    const response = new Response(null, { status: 502, statusText: "Bad Gateway" });
    vi.spyOn(response, "text").mockRejectedValue(new Error("stream failed"));

    await expect(ApiError.fromResponse(response)).resolves.toMatchObject({
      status: 502,
      code: "HTTP_502",
      message: "Bad Gateway",
    });
  });

  it("does not treat a legacy error message as a machine code", async () => {
    const response = new Response(JSON.stringify({ error: "Something went wrong" }), {
      status: 400,
      statusText: "Bad Request",
    });

    await expect(ApiError.fromResponse(response)).resolves.toMatchObject({
      code: "HTTP_400",
      message: "Something went wrong",
    });
  });

  it("treats a nested envelope as authoritative for its code", async () => {
    const response = new Response(
      JSON.stringify({ error: { message: "Nested" }, code: "LEGACY_CODE" }),
      { status: 409, statusText: "Conflict" },
    );

    await expect(ApiError.fromResponse(response)).resolves.toMatchObject({
      code: "HTTP_409",
      message: "Nested",
    });
  });
});
