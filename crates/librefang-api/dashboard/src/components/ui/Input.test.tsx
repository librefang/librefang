import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Input } from "./Input";

describe("Input", () => {
  it("keeps error colors for focus and hover states", () => {
    render(<Input label="Name" error="Name is required" />);

    const input = screen.getByRole("textbox", { name: "Name" });
    expect(input).toHaveClass(
      "border-red-500",
      "focus:border-red-500",
      "focus:ring-red-500/10",
      "hover:border-red-500",
    );
    expect(input).not.toHaveClass(
      "focus:border-brand",
      "focus:ring-brand/10",
      "hover:border-brand/20",
    );
  });

  it("keeps brand focus colors when there is no error", () => {
    render(<Input label="Name" />);

    const input = screen.getByRole("textbox", { name: "Name" });
    expect(input).toHaveClass(
      "focus:border-brand",
      "focus:ring-brand/10",
      "hover:border-brand/20",
    );
    expect(input).not.toHaveClass("focus:border-red-500");
  });
});
