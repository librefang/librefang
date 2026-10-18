import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useRovingTabs } from "./useRovingTabs";

const IDS = ["one", "two", "three"] as const;

function Harness({
  activeId,
  onSelect = () => {},
}: {
  activeId: string;
  onSelect?: (id: string) => void;
}) {
  const nav = useRovingTabs({
    ids: IDS,
    activeId: activeId as (typeof IDS)[number],
    onSelect: onSelect as (id: (typeof IDS)[number]) => void,
  });
  return (
    <div role="tablist" aria-label="Test tabs">
      {IDS.map((id) => (
        <button
          key={id}
          role="tab"
          aria-selected={id === activeId}
          tabIndex={nav.tabIndexFor(id)}
          onKeyDown={(event) => nav.onKeyDown(event, id)}
        >
          {id}
        </button>
      ))}
    </div>
  );
}

const tabs = () => screen.getAllByRole("tab");

describe("useRovingTabs", () => {
  it("gives the selected tab the only tab stop", () => {
    render(<Harness activeId="two" />);
    const stops = tabs().filter((tab) => tab.tabIndex === 0);
    expect(stops).toHaveLength(1);
    expect(stops[0]).toHaveTextContent("two");
    for (const tab of tabs()) {
      if (tab !== stops[0]) expect(tab.tabIndex).toBe(-1);
    }
  });

  it("anchors the tab stop on the first tab when the selection is missing", () => {
    render(<Harness activeId="gone" />);
    // A strip whose active id was pruned must not lose its single tab stop,
    // or the whole strip becomes keyboard-unreachable.
    expect(tabs()[0].tabIndex).toBe(0);
    expect(tabs()[1].tabIndex).toBe(-1);
    expect(tabs()[2].tabIndex).toBe(-1);
  });

  it("moves selection and focus with ArrowRight, wrapping at the end", () => {
    const onSelect = vi.fn();
    render(<Harness activeId="three" onSelect={onSelect} />);
    tabs()[2].focus();

    fireEvent.keyDown(tabs()[2], { key: "ArrowRight" });

    expect(onSelect).toHaveBeenCalledWith("one");
    expect(document.activeElement).toBe(tabs()[0]);
  });

  it("moves selection and focus with ArrowLeft, wrapping at the start", () => {
    const onSelect = vi.fn();
    render(<Harness activeId="one" onSelect={onSelect} />);
    tabs()[0].focus();

    fireEvent.keyDown(tabs()[0], { key: "ArrowLeft" });

    expect(onSelect).toHaveBeenCalledWith("three");
    expect(document.activeElement).toBe(tabs()[2]);
  });

  it("jumps to the edges with Home and End", () => {
    const onSelect = vi.fn();
    render(<Harness activeId="two" onSelect={onSelect} />);

    fireEvent.keyDown(tabs()[1], { key: "End" });
    expect(onSelect).toHaveBeenLastCalledWith("three");
    expect(document.activeElement).toBe(tabs()[2]);

    fireEvent.keyDown(tabs()[2], { key: "Home" });
    expect(onSelect).toHaveBeenLastCalledWith("one");
    expect(document.activeElement).toBe(tabs()[0]);
  });

  it("does not activate when the target is already selected", () => {
    const onSelect = vi.fn();
    render(<Harness activeId="one" onSelect={onSelect} />);

    fireEvent.keyDown(tabs()[0], { key: "Home" });

    expect(onSelect).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(tabs()[0]);
  });

  it("leaves other keys to the browser", () => {
    const onSelect = vi.fn();
    render(<Harness activeId="one" onSelect={onSelect} />);

    fireEvent.keyDown(tabs()[0], { key: "Tab" });
    fireEvent.keyDown(tabs()[0], { key: "Enter" });

    expect(onSelect).not.toHaveBeenCalled();
  });
});
