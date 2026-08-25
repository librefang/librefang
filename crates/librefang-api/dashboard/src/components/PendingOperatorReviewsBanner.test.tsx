import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PendingOperatorReviewsBanner } from "./PendingOperatorReviewsBanner";
import { usePendingOperatorRuns } from "../lib/queries/workflows";

vi.mock("../lib/queries/workflows", () => ({
  usePendingOperatorRuns: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => {
      if (key === "workflows.operator.review_run") {
        return `Review ${options?.workflow} — ${options?.step}`;
      }
      if (key === "workflows.operator.more_pending") {
        return `${options?.count} more pending`;
      }
      return key;
    },
  }),
}));

const usePendingOperatorRunsMock =
  usePendingOperatorRuns as unknown as ReturnType<typeof vi.fn>;

function row(index: number) {
  return {
    run_id: `run-${index}`,
    workflow_id: `workflow-${index}`,
    workflow_name: `Flow ${index}`,
    step_name: `Review ${index}`,
    operator_step_index: index,
    artifact: `Artifact ${index}`,
    actions: ["approve"],
    started_at: "2026-08-15T00:00:00Z",
    paused_at: "2026-08-15T00:01:00Z",
  };
}

describe("PendingOperatorReviewsBanner", () => {
  it("surfaces load failures and retries on demand", () => {
    const refetch = vi.fn();
    usePendingOperatorRunsMock.mockReturnValue({
      isLoading: false,
      isError: true,
      isFetching: false,
      data: undefined,
      refetch,
    });

    render(<PendingOperatorReviewsBanner />);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "workflows.operator.load_failed",
    );
    fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("bounds the visible backlog and tolerates missing actions", () => {
    const rows = Array.from({ length: 7 }, (_, index) => row(index));
    rows[0] = { ...rows[0], actions: null as unknown as string[] };
    usePendingOperatorRunsMock.mockReturnValue({
      isLoading: false,
      isError: false,
      isFetching: false,
      data: rows,
    });

    render(<PendingOperatorReviewsBanner />);
    expect(screen.getAllByRole("listitem")).toHaveLength(5);
    expect(screen.getByText("0 actions")).toBeInTheDocument();
    expect(screen.getByText("2 more pending")).toBeInTheDocument();
  });

  it("names a review action and forwards both identifiers", () => {
    const onSelectRun = vi.fn();
    usePendingOperatorRunsMock.mockReturnValue({
      isLoading: false,
      isError: false,
      isFetching: false,
      data: [row(3)],
    });

    render(<PendingOperatorReviewsBanner onSelectRun={onSelectRun} />);
    fireEvent.click(screen.getByRole("button", {
      name: "Review Flow 3 — Review 3",
    }));
    expect(onSelectRun).toHaveBeenCalledWith("run-3", "workflow-3");
  });
});
