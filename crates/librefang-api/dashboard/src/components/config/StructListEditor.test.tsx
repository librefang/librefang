import { StrictMode, useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { StructListEditor } from "./StructListEditor";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback: string) => fallback,
  }),
}));

vi.mock("../../lib/store", () => {
  let nextId = 0;
  return { createClientId: () => `row-${nextId++}` };
});

describe("StructListEditor", () => {
  it("keeps an expanded row mounted after a valid JSON edit", () => {
    function ControlledEditor() {
      const [items, setItems] = useState<unknown[]>([{ name: "before" }]);
      return <StructListEditor value={items} onChange={setItems} />;
    }

    render(
      <StrictMode>
        <ControlledEditor />
      </StrictMode>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Expand" }));
    const textarea = screen.getByRole("textbox");
    textarea.focus();

    fireEvent.change(textarea, {
      target: { value: '{"name":"after"}' },
    });

    expect(screen.getByRole("textbox")).toBe(textarea);
    expect(textarea).toHaveFocus();
    expect(screen.getByRole("button", { name: "Collapse" })).toBeInTheDocument();
  });

  it("keeps a later row mounted when an earlier sibling is removed", () => {
    function ControlledEditor() {
      const [items, setItems] = useState<unknown[]>([
        { name: "first" },
        { name: "second" },
      ]);
      return <StructListEditor value={items} onChange={setItems} />;
    }

    render(<ControlledEditor />);

    fireEvent.click(screen.getAllByRole("button", { name: "Expand" })[1]!);
    const textarea = screen.getByRole("textbox");
    fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]!);

    expect(screen.getByRole("textbox")).toBe(textarea);
    expect(screen.getByText("second")).toBeInTheDocument();
  });

  it("keeps an empty textarea as a draft instead of replacing the item with an object", () => {
    const onChange = vi.fn();
    render(<StructListEditor value={[{ name: "before" }]} onChange={onChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Expand" }));
    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "" } });

    expect(textarea).toHaveValue("");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.blur(textarea);
    expect(textarea).toHaveValue('{\n  "name": "before"\n}');
    expect(onChange).not.toHaveBeenCalled();
  });

  it("preserves valid compact JSON until blur despite the parent echo", () => {
    function ControlledEditor() {
      const [items, setItems] = useState<unknown[]>([{ name: "before" }]);
      return <StructListEditor value={items} onChange={setItems} />;
    }
    render(<ControlledEditor />);
    fireEvent.click(screen.getByRole("button", { name: "Expand" }));
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: '{"name":"after"}' } });

    expect(textarea).toHaveValue('{"name":"after"}');
    fireEvent.blur(textarea);
    expect(textarea).toHaveValue('{\n  "name": "after"\n}');
  });
});
