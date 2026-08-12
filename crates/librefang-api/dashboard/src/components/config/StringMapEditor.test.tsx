import { StrictMode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { StringMapEditor } from "./StringMapEditor";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback: string) => fallback,
  }),
}));

vi.mock("../../lib/store", () => {
  let nextId = 0;
  return { createClientId: () => `row-${nextId++}` };
});

describe("StringMapEditor", () => {
  it("emits one parent change for one edit in StrictMode", () => {
    const onChange = vi.fn();
    render(
      <StrictMode>
        <StringMapEditor value={{ endpoint: "old" }} onChange={onChange} />
      </StrictMode>,
    );

    fireEvent.change(screen.getAllByRole("textbox")[0]!, {
      target: { value: "renamed" },
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenLastCalledWith({ renamed: "old" });
  });

  it("keeps an empty numeric draft while the user types a replacement", () => {
    const onChange = vi.fn();
    render(
      <StringMapEditor value={{ timeout: 100 }} onChange={onChange} valueType="number" />,
    );
    const input = screen.getByRole("spinbutton");

    fireEvent.change(input, { target: { value: "" } });
    expect(input).toHaveValue(null);
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: "50" } });
    expect(input).toHaveValue(50);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenLastCalledWith({ timeout: 50 });
  });

  it("restores the last committed number when an empty draft loses focus", () => {
    const onChange = vi.fn();
    render(
      <StringMapEditor value={{ timeout: 100 }} onChange={onChange} valueType="number" />,
    );
    const input = screen.getByRole("spinbutton");

    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);

    expect(input).toHaveValue(100);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("marks normalized duplicate keys instead of discarding one silently", () => {
    const onChange = vi.fn();
    render(
      <StringMapEditor value={{ first: "one", second: "two" }} onChange={onChange} />,
    );
    const inputs = screen.getAllByRole("textbox");

    fireEvent.change(inputs[2]!, { target: { value: " first " } });

    expect(inputs[0]).toHaveAttribute("aria-invalid", "true");
    expect(inputs[2]).toHaveAttribute("aria-invalid", "true");
    expect(screen.getAllByText("Duplicate key")).toHaveLength(2);
    expect(onChange).toHaveBeenLastCalledWith({ first: "two" });
  });
});
