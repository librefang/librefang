import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Sparkline } from "./Sparkline";

describe("Sparkline", () => {
  it("does not render a path for a single data point", () => {
    const { container } = render(<Sparkline data={[42]} />);

    expect(container.querySelector("svg")).toBeNull();
  });
});
