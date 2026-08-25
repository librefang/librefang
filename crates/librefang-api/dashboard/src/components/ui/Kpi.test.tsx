import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Kpi } from "./Kpi";

describe("Kpi", () => {
  it("inherits keyboard activation from clickable cards", () => {
    const onClick = vi.fn();
    render(<Kpi label="Requests" value={42} onClick={onClick} />);

    const kpi = screen.getByRole("button", { name: "Requests" });
    expect(kpi).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(kpi, { key: "Enter" });
    expect(onClick).toHaveBeenCalledOnce();
  });
});
