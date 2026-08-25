import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RootErrorBoundary } from "./RootErrorBoundary";

function BrokenChild(): never {
  throw new Error("render exploded");
}

describe("RootErrorBoundary", () => {
  it("renders a dependency-free recovery surface for render failures", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    render(
      <RootErrorBoundary>
        <BrokenChild />
      </RootErrorBoundary>,
    );

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("render exploded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reload" })).toBeInTheDocument();
    consoleError.mockRestore();
  });
});
