import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { A2APage } from "./A2APage";

const addToast = vi.fn();
const discover = vi.fn();

vi.mock("../lib/queries/network", () => ({
  useA2AAgents: () => ({
    data: [],
    isFetching: false,
    isLoading: false,
    refetch: vi.fn(),
  }),
}));
vi.mock("../lib/mutations/network", () => ({
  useDiscoverA2AAgent: () => ({ mutateAsync: discover, isPending: false }),
  useSendA2ATask: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock("../lib/http/client", () => ({ getA2ATaskStatus: vi.fn() }));
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: { addToast: typeof addToast }) => unknown) =>
    selector({ addToast }),
}));
vi.mock("../lib/useCreateShortcut", () => ({ useCreateShortcut: vi.fn() }));

describe("A2APage", () => {
  beforeEach(() => {
    addToast.mockClear();
    discover.mockReset();
  });

  it("surfaces discovery failures", async () => {
    discover.mockRejectedValueOnce(new Error("discovery unavailable"));
    render(<A2APage />);

    fireEvent.click(screen.getAllByRole("button", { name: "a2a.discover" })[0]);
    fireEvent.change(screen.getByPlaceholderText("a2a.discover_placeholder"), {
      target: { value: "https://agent.example" },
    });
    fireEvent.click(screen.getAllByRole("button", { name: "a2a.discover" })[1]);

    await waitFor(() => {
      expect(addToast).toHaveBeenCalledWith("discovery unavailable", "error");
    });
  });
});
