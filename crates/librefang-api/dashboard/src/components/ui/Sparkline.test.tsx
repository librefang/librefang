import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Sparkline } from "./Sparkline";

describe("Sparkline", () => {
  it("does not render a path for a single data point", () => {
    const { container } = render(<Sparkline data={[42]} />);

    expect(container.querySelector("svg")).toBeNull();
  });

  it("renders data sets larger than the JavaScript argument limit", () => {
    const data = Array.from({ length: 150_000 }, (_, index) => index % 100);

    expect(() => render(<Sparkline data={data} />)).not.toThrow();
  });
});
