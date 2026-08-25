import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { EveryApiPartnerLink } from "./EveryApiPartnerLink";

describe("EveryApiPartnerLink", () => {
  it("uses the visible text as the expanded link name", () => {
    render(<EveryApiPartnerLink collapsed={false} />);

    const link = screen.getByRole("link", {
      name: "partner.official partner.librefang_everyapi",
    });
    expect(link).not.toHaveAttribute("aria-label");
  });

  it("provides a name when the visible label is collapsed", () => {
    render(<EveryApiPartnerLink collapsed />);

    expect(screen.getByRole("link", { name: "partner.everyapi_label" }))
      .toHaveAttribute("aria-label", "partner.everyapi_label");
  });
});
