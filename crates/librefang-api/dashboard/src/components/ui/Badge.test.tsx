import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Badge } from "./Badge";

describe("Badge", () => {
  it("lets className override conflicting default and variant utilities", () => {
    render(
      <Badge variant="success" className="px-4 text-sm bg-error/10">
        Offline
      </Badge>,
    );

    const badge = screen.getByText("Offline");
    expect(badge).toHaveClass("px-4", "text-sm", "bg-error/10");
    expect(badge).not.toHaveClass("px-2", "text-[10px]", "bg-success/10");
  });
});
