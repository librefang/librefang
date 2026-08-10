import { StrictMode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { StringMapEditor } from "./StringMapEditor";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback: string) => fallback,
  }),
}));

vi.mock("../../lib/store", () => ({
  createClientId: () => "row-id",
}));

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
});
