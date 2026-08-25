import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Pill } from "./Pill";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe("Pill", () => {
  it("uses the denied translation instead of the rejected status label", () => {
    render(<Pill kind="denied" />);

    expect(
      screen.getByText("approvals.history.decisions.denied"),
    ).toBeInTheDocument();
    expect(screen.queryByText("approvals.status.rejected")).not.toBeInTheDocument();
  });

  it("preserves explicit zero content", () => {
    render(<Pill kind="idle">{0}</Pill>);

    expect(screen.getByText("0")).toBeInTheDocument();
    expect(screen.queryByText("common.idle")).not.toBeInTheDocument();
  });
});
