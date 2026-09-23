import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { CollapsibleSection } from "./CollapsibleSection";

describe("CollapsibleSection", () => {
  it("starts folded so a long form does not open as a wall", () => {
    render(
      <CollapsibleSection title="Routing">
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(screen.getByText("Routing").closest("details")).not.toHaveAttribute("open");
  });

  it("opens when asked", () => {
    render(
      <CollapsibleSection title="Routing" defaultOpen>
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(screen.getByText("Routing").closest("details")).toHaveAttribute("open");
  });

  it("forces itself open when invalid, because a hidden error reads as no error", () => {
    render(
      <CollapsibleSection title="Routing" invalid>
        <p>body</p>
      </CollapsibleSection>,
    );
    const details = screen.getByText("Routing").closest("details");
    expect(details).toHaveAttribute("open");
    expect(screen.getByText("Routing").closest("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("is a real details/summary, so the keyboard and toggle behaviour come free", () => {
    const { container } = render(
      <CollapsibleSection title="Routing">
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(container.querySelector("details")).not.toBeNull();
    expect(container.querySelector("summary")).not.toBeNull();
  });

  it("keeps the body mounted while folded, so form state survives", () => {
    render(
      <CollapsibleSection title="Routing">
        <input aria-label="threshold" defaultValue="100" />
      </CollapsibleSection>,
    );
    // The value is still in the DOM: folding must not unmount a control, or
    // the field would silently reset every time the section was collapsed.
    expect(screen.getByLabelText("threshold")).toHaveValue("100");
  });
});
