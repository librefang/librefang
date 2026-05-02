import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RuntimePage } from "./RuntimePage";
import {
  useQueueStatus,
  useHealthDetail,
  useSecurityStatus,
  useAuditRecent,
  useAuditVerify,
  useBackups,
  useTaskQueueStatus,
  useTaskQueue,
} from "../lib/queries/runtime";
import {
  useDashboardSnapshot,
  useVersionInfo,
} from "../lib/queries/overview";
import {
  useShutdownServer,
  useCreateBackup,
  useRestoreBackup,
  useDeleteBackup,
  useDeleteTask,
  useRetryTask,
  useCleanupSessions,
} from "../lib/mutations/runtime";
import { useReloadConfig } from "../lib/mutations/config";

vi.mock("../lib/queries/runtime", () => ({
  useQueueStatus: vi.fn(),
  useHealthDetail: vi.fn(),
  useSecurityStatus: vi.fn(),
  useAuditRecent: vi.fn(),
  useAuditVerify: vi.fn(),
  useBackups: vi.fn(),
  useTaskQueueStatus: vi.fn(),
  useTaskQueue: vi.fn(),
}));

vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: vi.fn(),
  useVersionInfo: vi.fn(),
}));

vi.mock("../lib/mutations/runtime", () => ({
  useShutdownServer: vi.fn(),
  useCreateBackup: vi.fn(),
  useRestoreBackup: vi.fn(),
  useDeleteBackup: vi.fn(),
  useDeleteTask: vi.fn(),
  useRetryTask: vi.fn(),
  useCleanupSessions: vi.fn(),
}));

vi.mock("../lib/mutations/config", () => ({
  useReloadConfig: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

// Cast helpers
const m = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;

const useQueueStatusMock = m(useQueueStatus);
const useHealthDetailMock = m(useHealthDetail);
const useSecurityStatusMock = m(useSecurityStatus);
const useAuditRecentMock = m(useAuditRecent);
const useAuditVerifyMock = m(useAuditVerify);
const useBackupsMock = m(useBackups);
const useTaskQueueStatusMock = m(useTaskQueueStatus);
const useTaskQueueMock = m(useTaskQueue);
const useDashboardSnapshotMock = m(useDashboardSnapshot);
const useVersionInfoMock = m(useVersionInfo);
const useShutdownServerMock = m(useShutdownServer);
const useCreateBackupMock = m(useCreateBackup);
const useRestoreBackupMock = m(useRestoreBackup);
const useDeleteBackupMock = m(useDeleteBackup);
const useDeleteTaskMock = m(useDeleteTask);
const useRetryTaskMock = m(useRetryTask);
const useCleanupSessionsMock = m(useCleanupSessions);
const useReloadConfigMock = m(useReloadConfig);

function makeQuery<T>(data: T, overrides: Record<string, unknown> = {}) {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    isSuccess: data !== undefined,
    refetch: vi.fn().mockResolvedValue({ data, isSuccess: true, isError: false }),
    ...overrides,
  };
}

function makeMutation(overrides: Record<string, unknown> = {}) {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    isSuccess: false,
    isError: false,
    data: undefined,
    error: null,
    ...overrides,
  };
}

function setQueryDefaults() {
  useDashboardSnapshotMock.mockReturnValue(
    makeQuery({
      status: {
        version: "2026.5.1",
        active_agent_count: 2,
        agent_count: 3,
        session_count: 4,
        uptime_seconds: 7200,
        memory_used_mb: 128,
        default_provider: "openai",
        default_model: "gpt-4",
        api_listen: "127.0.0.1:4545",
        home_dir: "/home/x/.librefang",
        log_level: "info",
        network_enabled: true,
      },
      providers: [{ id: "openai", auth_status: "ok" }],
      channels: [{ id: "telegram", configured: true }],
      skillCount: 5,
      workflowCount: 1,
      health: {
        status: "ok",
        checks: [{ name: "db", status: "ok" }],
      },
    }),
  );
  useVersionInfoMock.mockReturnValue(
    makeQuery({
      version: "2026.5.1",
      git_sha: "abc123def456",
      build_date: "2026-05-01",
      rust_version: "1.85",
      platform: "linux",
      arch: "x86_64",
      hostname: "node-1",
    }),
  );
  useQueueStatusMock.mockReturnValue(
    makeQuery({
      lanes: [{ lane: "main", active: 1, capacity: 4 }],
      config: { max_depth_per_agent: 16, max_depth_global: 64, task_ttl_secs: 600 },
    }),
  );
  useHealthDetailMock.mockReturnValue(
    makeQuery({
      database: "connected",
      memory: { embedding_available: false, proactive_memory_enabled: false },
      panic_count: 0,
      restart_count: 0,
      config_warnings: [],
    }),
  );
  useSecurityStatusMock.mockReturnValue(
    makeQuery({
      total_features: 7,
      core_protections: { sandbox: true, signing: true },
      configurable: { auth: { mode: "token" } },
      monitoring: {
        audit_trail: { enabled: true, algorithm: "sha256" },
      },
    }),
  );
  useAuditRecentMock.mockReturnValue(
    makeQuery({
      entries: [
        {
          seq: 1,
          outcome: "ok",
          action: "agent.start",
          agent_id: "agent-1234abcd",
          timestamp: "2026-05-01T12:00:00Z",
          detail: "started",
        },
      ],
    }),
  );
  useAuditVerifyMock.mockReturnValue(makeQuery({ valid: true, entries: 42 }));
  useBackupsMock.mockReturnValue(
    makeQuery({
      backups: [
        { filename: "backup-1.tar.gz", size_bytes: 2048, created_at: "2026-05-01T00:00:00Z" },
      ],
    }),
  );
  useTaskQueueStatusMock.mockReturnValue(
    makeQuery({ total: 5, pending: 1, in_progress: 1, completed: 3, failed: 0 }),
  );
  useTaskQueueMock.mockReturnValue(
    makeQuery({
      tasks: [
        { id: "task-failed-1", status: "failed", type: "llm", created_at: "2026-05-01" },
        { id: "task-pending-2", status: "pending", type: "llm", created_at: "2026-05-01" },
      ],
    }),
  );
}

function setMutationDefaults() {
  useShutdownServerMock.mockReturnValue(makeMutation());
  useCreateBackupMock.mockReturnValue(makeMutation());
  useRestoreBackupMock.mockReturnValue(makeMutation());
  useDeleteBackupMock.mockReturnValue(makeMutation());
  useDeleteTaskMock.mockReturnValue(makeMutation());
  useRetryTaskMock.mockReturnValue(makeMutation());
  useCleanupSessionsMock.mockReturnValue(makeMutation());
  useReloadConfigMock.mockReturnValue(makeMutation());
}

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <RuntimePage />
    </QueryClientProvider>,
  );
}

describe("RuntimePage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setQueryDefaults();
    setMutationDefaults();
  });

  it("shows skeleton placeholders while the snapshot is loading", () => {
    useDashboardSnapshotMock.mockReturnValue(
      makeQuery(undefined, { isLoading: true, isFetching: true }),
    );
    renderPage();
    // CardSkeleton renders role=status with aria-busy=true while loading.
    expect(screen.getAllByRole("status").length).toBeGreaterThan(0);
    expect(screen.queryByText("runtime.engine")).not.toBeInTheDocument();
  });

  it("renders the error card and retry button when snapshot fails", () => {
    useDashboardSnapshotMock.mockReturnValue(
      makeQuery(undefined, { isError: true, isLoading: false }),
    );
    renderPage();
    expect(screen.getByText("runtime.load_error")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "common.retry" })).toBeInTheDocument();
  });

  it("renders KPI tiles, engine info, and config rows on happy path", () => {
    renderPage();
    // KPI labels
    expect(screen.getByText("runtime.system_uptime")).toBeInTheDocument();
    expect(screen.getByText("runtime.active_agents")).toBeInTheDocument();
    // Active/total agents
    expect(screen.getByText("2 / 3")).toBeInTheDocument();
    // Engine + config sections
    expect(screen.getByText("runtime.engine")).toBeInTheDocument();
    expect(screen.getByText("runtime.config")).toBeInTheDocument();
    // Version pulled from version query
    expect(screen.getByText("2026.5.1")).toBeInTheDocument();
    // Config values
    expect(screen.getByText("openai")).toBeInTheDocument();
    expect(screen.getByText("gpt-4")).toBeInTheDocument();
  });

  it("renders audit entries with action and validity badge", () => {
    renderPage();
    expect(screen.getByText("agent.start")).toBeInTheDocument();
    expect(screen.getByText("runtime.audit_valid")).toBeInTheDocument();
  });

  it("renders backups list with filename and size", () => {
    renderPage();
    expect(screen.getByText("backup-1.tar.gz")).toBeInTheDocument();
    // formatBytes(2048) → "2.0 KB"
    expect(screen.getByText(/2\.0 KB/)).toBeInTheDocument();
  });

  it("opens shutdown confirm dialog and fires shutdown mutation on confirm", () => {
    const mutate = vi.fn();
    useShutdownServerMock.mockReturnValue(makeMutation({ mutate }));
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "runtime.shutdown" }));
    // Confirm dialog now visible
    const confirmBtn = screen.getByRole("button", { name: "runtime.shutdown_confirm" });
    fireEvent.click(confirmBtn);
    expect(mutate).toHaveBeenCalledTimes(1);
  });

  it("invokes reloadConfig mutation when reload button is clicked", () => {
    const mutate = vi.fn();
    useReloadConfigMock.mockReturnValue(makeMutation({ mutate }));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "runtime.reload_config" }));
    expect(mutate).toHaveBeenCalledTimes(1);
  });

  it("invokes createBackup mutation when create backup button is clicked", () => {
    const mutate = vi.fn();
    useCreateBackupMock.mockReturnValue(makeMutation({ mutate }));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "runtime.create_backup" }));
    expect(mutate).toHaveBeenCalledTimes(1);
  });

  it("retries a failed task via retryTask mutation", () => {
    const mutate = vi.fn();
    useRetryTaskMock.mockReturnValue(makeMutation({ mutate }));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "runtime.retry" }));
    expect(mutate).toHaveBeenCalledWith("task-failed-1");
  });

  it("opens restore confirm dialog and fires restoreBackup with filename on confirm", () => {
    const mutate = vi.fn();
    useRestoreBackupMock.mockReturnValue(makeMutation({ mutate }));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "runtime.restore" }));
    fireEvent.click(screen.getByRole("button", { name: "runtime.restore_confirm" }));
    expect(mutate).toHaveBeenCalledWith("backup-1.tar.gz");
  });
});
