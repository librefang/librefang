// Tests for the Memory Health tab's handling of queries that have not answered yet or have failed.
// A missing answer must not be rendered as a negative diagnosis ("Not reachable", "Disabled").

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { HealthTab } from "./HealthTab";
import { useMemoryConfig, useMemoryHealth } from "../../../lib/queries/memory";
import { useAutoDreamStatus } from "../../../lib/queries/autoDream";

vi.mock("../../../lib/queries/memory", () => ({
  useMemoryConfig: vi.fn(),
  useMemoryHealth: vi.fn(),
}));

vi.mock("../../../lib/queries/autoDream", () => ({
  useAutoDreamStatus: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    i18n: { language: "en" },
  }),
}));

const useMemoryConfigMock = useMemoryConfig as unknown as ReturnType<typeof vi.fn>;
const useMemoryHealthMock = useMemoryHealth as unknown as ReturnType<typeof vi.fn>;
const useAutoDreamStatusMock = useAutoDreamStatus as unknown as ReturnType<typeof vi.fn>;

const pending = () => ({ data: undefined, isPending: true, isError: false, error: null, refetch: vi.fn() });
const failed = (message: string) => ({
  data: undefined,
  isPending: false,
  isError: true,
  error: new Error(message),
  refetch: vi.fn(),
});

describe("HealthTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows a neutral loading state, not negative facts, while the queries are in flight", () => {
    useMemoryHealthMock.mockReturnValue(pending());
    useMemoryConfigMock.mockReturnValue(pending());
    useAutoDreamStatusMock.mockReturnValue(pending());

    render(<HealthTab onOpenConfig={vi.fn()} />);

    expect(screen.queryByText("Not reachable")).not.toBeInTheDocument();
    expect(screen.queryByText(/Liveness probe failed/)).not.toBeInTheDocument();
    expect(screen.queryByText("Disabled")).not.toBeInTheDocument();
    expect(screen.queryByText("auto-detect")).not.toBeInTheDocument();
    expect(screen.getAllByText("Loading...")).toHaveLength(3);
  });

  it("shows the load error with a retry instead of 'Not reachable' / 'Disabled' when the queries fail", () => {
    const health = failed("health boom");
    const config = failed("config boom");
    const dream = failed("dream boom");
    useMemoryHealthMock.mockReturnValue(health);
    useMemoryConfigMock.mockReturnValue(config);
    useAutoDreamStatusMock.mockReturnValue(dream);

    render(<HealthTab onOpenConfig={vi.fn()} />);

    expect(screen.queryByText("Not reachable")).not.toBeInTheDocument();
    expect(screen.queryByText(/Liveness probe failed/)).not.toBeInTheDocument();
    expect(screen.queryByText("Disabled")).not.toBeInTheDocument();
    expect(screen.getByText("health boom")).toBeInTheDocument();
    expect(screen.getByText("config boom")).toBeInTheDocument();
    expect(screen.getByText("dream boom")).toBeInTheDocument();

    const retries = screen.getAllByRole("button", { name: "Retry" });
    expect(retries).toHaveLength(3);
    retries.forEach((b) => fireEvent.click(b));
    expect(health.refetch).toHaveBeenCalledTimes(1);
    expect(config.refetch).toHaveBeenCalledTimes(1);
    expect(dream.refetch).toHaveBeenCalledTimes(1);
  });

  it("still reports a failed liveness probe and disabled features once the data says so", () => {
    useMemoryHealthMock.mockReturnValue({ data: false, isPending: false, isError: false, refetch: vi.fn() });
    useMemoryConfigMock.mockReturnValue({
      data: { proactive_memory: { enabled: false } },
      isPending: false,
      isError: false,
      refetch: vi.fn(),
    });
    useAutoDreamStatusMock.mockReturnValue({
      data: { enabled: false, agents: [] },
      isPending: false,
      isError: false,
      refetch: vi.fn(),
    });

    render(<HealthTab onOpenConfig={vi.fn()} />);

    expect(screen.getByText("Not reachable")).toBeInTheDocument();
    expect(screen.getByText(/Liveness probe failed/)).toBeInTheDocument();
    expect(screen.getAllByText("Disabled")).toHaveLength(2);
    expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
  });
});
