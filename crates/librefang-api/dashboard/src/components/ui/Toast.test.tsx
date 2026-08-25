import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useUIStore } from "../../lib/store";
import { ToastContainer } from "./Toast";

vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  }),
}));

describe("ToastContainer", () => {
  beforeEach(() => {
    act(() =>
      useUIStore.setState({
        toasts: [
          { id: "success", message: "Saved", type: "success" },
          { id: "error", message: "Failed", type: "error" },
        ],
      }),
    );
  });

  afterEach(() => {
    act(() => useUIStore.setState({ toasts: [] }));
  });

  it("lets each toast role define its own announcement priority", () => {
    render(<ToastContainer />);

    const status = screen.getByRole("status");
    const alert = screen.getByRole("alert");
    const container = status.parentElement;

    expect(status).toHaveTextContent("Saved");
    expect(alert).toHaveTextContent("Failed");
    expect(container).not.toHaveAttribute("aria-live");
    expect(container).not.toHaveAttribute("aria-atomic");
  });
});
