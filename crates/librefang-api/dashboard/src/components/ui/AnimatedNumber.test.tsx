import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AnimatedNumber } from "./AnimatedNumber";

describe("AnimatedNumber", () => {
  it.each(["1.234.56", "12-3", "1,5", "$1,234.50"])(
    "renders the invalid numeric string %s unchanged",
    (value) => {
      render(<AnimatedNumber value={value} />);

      expect(screen.getByText(value)).toBeInTheDocument();
    },
  );

  it("animates a complete decimal string", () => {
    render(<AnimatedNumber value="-42.5" decimals={1} />);

    expect(screen.getByText("-42.5")).toBeInTheDocument();
  });

  it("renders non-finite numbers unchanged", () => {
    render(<AnimatedNumber value={Number.POSITIVE_INFINITY} />);

    expect(screen.getByText("Infinity")).toBeInTheDocument();
  });
});
