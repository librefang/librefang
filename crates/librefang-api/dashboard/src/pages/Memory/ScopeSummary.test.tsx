import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AutoDreamAgentStatus, AutoDreamStatus } from "../../api";
import { ScopeSummary } from "./ScopeSummary";

const tMock = vi.fn((key: string, options?: { defaultValue?: string }) =>
  options?.defaultValue ?? key
);

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: tMock,
    i18n: { language: "en" },
  }),
}));

const now = new Date("2026-08-15T12:00:00Z").getTime();
const dreamForAgent: AutoDreamAgentStatus = {
  agent_id: "agent-1",
  agent_name: "Planner",
  auto_dream_enabled: true,
  last_consolidated_at_ms: now - 61_000,
  next_eligible_at_ms: now + 61_000,
  hours_since_last: 1,
  sessions_since_last: 2,
  effective_min_hours: 1,
  effective_min_sessions: 1,
  lock_path: "/tmp/agent-1.lock",
  progress: null,
  can_abort: false,
};
const autoDream: AutoDreamStatus = {
  enabled: true,
  min_hours: 1,
  min_sessions: 1,
  check_interval_secs: 30,
  lock_dir: "/tmp",
  agents: [dreamForAgent],
};

function renderSummary() {
  return render(
    <ScopeSummary
      scopedAgent={{ id: "agent-1", name: "Planner" }}
      agentStats={{ total: 3, user_count: 1, session_count: 1, agent_count: 1 }}
      autoDream={autoDream}
      dreamForAgent={dreamForAgent}
      kvCount={2}
    />,
  );
}

afterEach(() => {
  vi.useRealTimers();
  tMock.mockClear();
});

describe("ScopeSummary", () => {
  it("uses Memory-owned Auto-Dream labels with readable fallbacks", () => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
    renderSummary();

    expect(screen.getByText("Enrolled")).toBeInTheDocument();
    expect(tMock).toHaveBeenCalledWith("memory.auto_dream_enrolled", {
      defaultValue: "Enrolled",
    });
    expect(tMock).toHaveBeenCalledWith("memory.auto_dream_last", { defaultValue: "Last" });
    expect(tMock).toHaveBeenCalledWith("memory.auto_dream_next", { defaultValue: "Next" });
    expect(tMock.mock.calls.some(([key]) => key.startsWith("settings."))).toBe(false);
  });

  it("refreshes relative-time labels every 30 seconds and clears the timer", () => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
    const clearIntervalSpy = vi.spyOn(window, "clearInterval");
    const { unmount } = renderSummary();
    const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto", style: "narrow" });

    expect(screen.getByText(rtf.format(-1, "minute"), { exact: false })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(30_000));
    expect(screen.getByText(rtf.format(-2, "minute"), { exact: false })).toBeInTheDocument();

    unmount();
    expect(clearIntervalSpy).toHaveBeenCalledOnce();
    clearIntervalSpy.mockRestore();
  });
});
