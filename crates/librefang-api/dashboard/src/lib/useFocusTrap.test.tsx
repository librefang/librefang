import { useRef, type ReactNode } from "react";
import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useFocusTrap } from "./useFocusTrap";

function Harness({ children }: { children?: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useFocusTrap(true, ref);
  return (
    <div ref={ref} tabIndex={-1} data-testid="trap">
      {children}
    </div>
  );
}

const pressTab = (shiftKey = false): KeyboardEvent => {
  const event = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  act(() => window.dispatchEvent(event));
  return event;
};

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(
    function getClientRects(this: HTMLElement) {
      return (this.dataset.hidden === "true" ? [] : [{}]) as unknown as DOMRectList;
    },
  );
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("useFocusTrap", () => {
  it("uses the same visibility filter for initial focus", () => {
    render(
      <Harness>
        <button data-hidden="true">Hidden</button>
        <button>Visible</button>
      </Harness>,
    );
    expect(screen.getByRole("button", { name: "Visible" })).toHaveFocus();
  });

  it("pulls escaped focus back to the first or last element", () => {
    const outside = document.createElement("button");
    document.body.append(outside);
    render(
      <Harness>
        <button>First</button>
        <button>Last</button>
      </Harness>,
    );

    outside.focus();
    expect(pressTab().defaultPrevented).toBe(true);
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();

    outside.focus();
    expect(pressTab(true).defaultPrevented).toBe(true);
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
  });

  it("recovers when focus is on a non-tabbable descendant", () => {
    render(
      <Harness>
        <button>First</button>
        <span tabIndex={-1}>Programmatic</span>
        <button>Last</button>
      </Harness>,
    );
    screen.getByText("Programmatic").focus();

    expect(pressTab().defaultPrevented).toBe(true);
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
  });

  it("keeps focus on the container when it has no focusable descendants", () => {
    render(<Harness />);
    const trap = screen.getByTestId("trap");
    expect(trap).toHaveFocus();

    expect(pressTab().defaultPrevented).toBe(true);
    expect(trap).toHaveFocus();
  });
});
