import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it.each([
    ["Jane Doe", "JD"],
    ["  Jane   Doe  ", "JD"],
    ["Jane\tDoe", "JD"],
    ["Jane\nDoe", "JD"],
    ["Jane\u00a0Doe", "JD"],
    ["Jane", "J"],
    ["   ", "?"],
  ])("derives initials from %j", (fallback, expected) => {
    render(<Avatar fallback={fallback} />);

    const avatar = screen.getByRole("img");
    expect(avatar).toHaveAttribute("aria-label", fallback);
    expect(avatar).toHaveTextContent(expected);
  });
});
