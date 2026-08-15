import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Card } from "./Card";

describe("Card", () => {
  it("keeps non-clickable cards non-interactive", () => {
    render(<Card>Summary</Card>);

    const card = screen.getByText("Summary");
    expect(card).not.toHaveAttribute("role");
    expect(card).not.toHaveAttribute("tabindex");
  });

  it.each(["Enter", " "])("activates a clickable card with %s", (key) => {
    const onClick = vi.fn();
    render(<Card onClick={onClick}>Open details</Card>);

    const card = screen.getByRole("button", { name: "Open details" });
    expect(card).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(card, { key });
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("honors a caller that prevents keyboard activation", () => {
    const onClick = vi.fn();
    render(
      <Card onClick={onClick} onKeyDown={(event) => event.preventDefault()}>
        Managed card
      </Card>,
    );

    fireEvent.keyDown(screen.getByRole("button"), { key: "Enter" });
    expect(onClick).not.toHaveBeenCalled();
  });
});
