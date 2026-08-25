import { fireEvent, render, screen } from "@testing-library/react";
import type { FormEvent } from "react";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./Button";

describe("Button", () => {
  it("does not submit a surrounding form by default", () => {
    const onSubmit = vi.fn((event: FormEvent) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button>Cancel</Button>
      </form>,
    );

    const button = screen.getByRole("button", { name: "Cancel" });
    expect(button).toHaveAttribute("type", "button");
    fireEvent.click(button);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("preserves an explicit submit type", () => {
    const onSubmit = vi.fn((event: FormEvent) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button type="submit">Save</Button>
      </form>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSubmit).toHaveBeenCalledOnce();
  });
});
