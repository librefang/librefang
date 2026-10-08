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

  it("stays open on recovery, so clearing one error does not collapse the section being edited", () => {
    const { rerender } = render(
      <CollapsibleSection title="Routing" invalid>
        <p>body</p>
      </CollapsibleSection>,
    );
    const details = screen.getByText("Routing").closest("details");
    expect(details).toHaveAttribute("open");

    // `invalid` going back to false must not yank the section shut — the
    // operator may have just fixed one of two errors and be mid-edit.
    rerender(
      <CollapsibleSection title="Routing" invalid={false}>
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(details).toHaveAttribute("open");
  });

  it("does not clip a child popover once open", () => {
    // `overflow-hidden` trims the body to the rounded corners while folded, but
    // it also clipped the absolutely-positioned model picker popover a child
    // renders. The clip is released once `open` is set on the element. The
    // `open:` variant matches the element itself; `group-open:` would compile
    // to a descendant selector and never match, so pin the working token.
    render(
      <CollapsibleSection title="Routing" defaultOpen>
        <p>body</p>
      </CollapsibleSection>,
    );
    const details = screen.getByText("Routing").closest("details")!;
    expect(details.className).toContain("open:overflow-visible");
    expect(details.className).not.toContain("group-open:overflow-visible");
  });

  it("keeps the clip while folded", () => {
    render(
      <CollapsibleSection title="Routing">
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(screen.getByText("Routing").closest("details")!.className).toContain("overflow-hidden");
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
