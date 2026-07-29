import { describe, expect, it } from "vitest";

import { EVERYAPI_PARTNER } from "./partner";

describe("EveryAPI partner links", () => {
  it("routes dashboard users to the joint partner page with attribution", () => {
    const url = new URL(EVERYAPI_PARTNER.pageUrl);

    expect(url.origin).toBe("https://everyapi.ai");
    expect(url.pathname).toBe("/integrations/librefang");
    expect(url.searchParams.get("utm_source")).toBe("librefang_dashboard");
    expect(url.searchParams.get("utm_medium")).toBe("partner");
  });

  it("keeps EveryAPI's public product destination canonical", () => {
    expect(EVERYAPI_PARTNER.websiteUrl).toBe("https://everyapi.ai/");
  });
});
