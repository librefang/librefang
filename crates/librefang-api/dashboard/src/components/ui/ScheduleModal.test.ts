import { describe, expect, it } from "vitest";
import { validateCron } from "./ScheduleModal";

describe("validateCron", () => {
  it("accepts supported values, lists, ranges, and steps", () => {
    expect(validateCron("0 9 * * *")).toBe(true);
    expect(validateCron("*/15 0,12 1-31/2 1,6,12 0-7")).toBe(true);
    expect(validateCron("1/2 2/3 * * 7")).toBe(true);
  });

  it.each([
    "60 * * * *",
    "* 24 * * *",
    "* * 0 * *",
    "* * 32 * *",
    "* * * 0 *",
    "* * * 13 *",
    "* * * * 8",
  ])("rejects out-of-range fields in %s", (cron) => {
    expect(validateCron(cron)).toBe(false);
  });

  it.each([
    "*/0 * * * *",
    "*/x * * * *",
    "1-0 * * * *",
    "1-2-3 * * * *",
    "1,,2 * * * *",
    "1/2/3 * * * *",
    "0 9 * *",
  ])("rejects malformed expressions in %s", (cron) => {
    expect(validateCron(cron)).toBe(false);
  });
});
