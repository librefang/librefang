import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { NodeEditor } from "./NodeEditor";

describe("NodeEditor", () => {
  it("syncs an externally updated label for the selected node", () => {
    const onUpdate = vi.fn();
    const { rerender } = render(
      <NodeEditor
        node={{ id: "node-1", type: "agent", data: { label: "Original" } }}
        onUpdate={onUpdate}
      />,
    );

    expect(screen.getByLabelText("common.label")).toHaveValue("Original");

    rerender(
      <NodeEditor
        node={{ id: "node-1", type: "agent", data: { label: "Updated externally" } }}
        onUpdate={onUpdate}
      />,
    );

    expect(screen.getByLabelText("common.label")).toHaveValue("Updated externally");
  });
});
