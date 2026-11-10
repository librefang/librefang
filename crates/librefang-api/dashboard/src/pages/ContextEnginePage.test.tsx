import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ContextEnginePage } from "./ContextEnginePage";
import {
  getContextEngineChain,
  getContextEngineConfig,
  getContextEngineHealth,
  getContextEngineMetrics,
  getContextEngineSandboxPolicy,
  getContextEngineTraces,
} from "../api";
import type {
  ContextEngineChain,
  ContextEngineConfigSnapshot,
  ContextEngineHookCoverage,
  ContextEngineHookStats,
  ContextEngineMetrics,
  ContextEngineSandboxPolicy,
  ContextEngineTraces,
} from "../api";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return { ...actual, useTranslation: () => ({ t: (key: string) => key }) };
});

vi.mock("../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api")>();
  return {
    ...actual,
    getContextEngineHealth: vi.fn(),
    getContextEngineChain: vi.fn(),
    getContextEngineMetrics: vi.fn(),
    getContextEngineTraces: vi.fn(),
    getContextEngineConfig: vi.fn(),
    getContextEngineSandboxPolicy: vi.fn(),
  };
});

const healthMock = vi.mocked(getContextEngineHealth);
const chainMock = vi.mocked(getContextEngineChain);
const metricsMock = vi.mocked(getContextEngineMetrics);
const tracesMock = vi.mocked(getContextEngineTraces);
const configMock = vi.mocked(getContextEngineConfig);
const sandboxMock = vi.mocked(getContextEngineSandboxPolicy);

const EMPTY_STATS: ContextEngineHookStats = { calls: 0, successes: 0, failures: 0, total_ms: 0 };

function metricsWith(overrides: Partial<ContextEngineMetrics> = {}): ContextEngineMetrics {
  return {
    ingest: EMPTY_STATS,
    after_turn: EMPTY_STATS,
    bootstrap: EMPTY_STATS,
    assemble: EMPTY_STATS,
    compact: EMPTY_STATS,
    transform_tool_result: EMPTY_STATS,
    prepare_subagent: EMPTY_STATS,
    merge_subagent: EMPTY_STATS,
    ...overrides,
  };
}

function coverage(overrides: Partial<ContextEngineHookCoverage> = {}): ContextEngineHookCoverage {
  return {
    ingest: false,
    after_turn: false,
    assemble: false,
    compact: false,
    bootstrap: false,
    prepare_subagent: false,
    merge_subagent: false,
    runtime: null,
    enabled: true,
    hooks_valid: true,
    cache_ttl_secs: null,
    ...overrides,
  };
}

const defaultChain: ContextEngineChain = {
  mode: "default",
  chain: [],
  chain_length: 0,
  fallback: "default engine (embedding recall)",
};

const defaultConfig: ContextEngineConfigSnapshot = {
  engine: "default",
  plugin: null,
  plugin_stack: null,
  hook_timeout_secs: 30,
  on_hook_failure: "warn",
  max_retries: 0,
  retry_delay_ms: 500,
  hook_cache_ttl_secs: null,
  assemble_cache_ttl_secs: null,
  compact_cache_ttl_secs: null,
  persistent_subprocess: false,
  prewarm_subprocesses: false,
  allow_network: false,
  allow_filesystem: false,
  max_memory_mb: null,
  after_turn_queue_depth: 0,
  circuit_breaker: null,
  otel_endpoint: null,
  registries: [],
};

const defaultSandbox: ContextEngineSandboxPolicy = {
  mode: "default",
  plugins: [],
  global_defaults: {
    allow_filesystem: false,
    allow_network: false,
    max_memory_mb: null,
    otel_endpoint: null,
  },
};

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ContextEnginePage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  healthMock.mockResolvedValue(null);
  chainMock.mockResolvedValue(defaultChain);
  metricsMock.mockResolvedValue(null);
  tracesMock.mockResolvedValue(null);
  configMock.mockResolvedValue(defaultConfig);
  sandboxMock.mockResolvedValue(defaultSandbox);
});

describe("ContextEnginePage", () => {
  it("renders the no-engine state when the 204 routes return null", async () => {
    renderPage();

    await screen.findByTestId("context-engine-health");
    expect(screen.getAllByText("context_engine.no_engine").length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText("context_engine.no_engine_desc")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("renders a healthy card with active plugins and no issues", async () => {
    healthMock.mockResolvedValue({
      healthy: true,
      active_plugins: ["memory-lens", "summarizer"],
      issues: [],
    });
    renderPage();

    const card = await screen.findByTestId("context-engine-health");
    expect(within(card).getByText("context_engine.health.healthy")).toBeInTheDocument();
    expect(within(card).getByText("memory-lens")).toBeInTheDocument();
    expect(within(card).getByText("summarizer")).toBeInTheDocument();
    expect(within(card).getByText("context_engine.health.no_issues")).toBeInTheDocument();
  });

  it("renders unhealthy status and the issue list", async () => {
    healthMock.mockResolvedValue({
      healthy: false,
      active_plugins: ["memory-lens"],
      issues: [{ plugin: "memory-lens", errors: ["missing ingest hook script"] }],
    });
    renderPage();

    const card = await screen.findByTestId("context-engine-health");
    expect(within(card).getByText("context_engine.health.unhealthy")).toBeInTheDocument();
    expect(within(card).getByText("missing ingest hook script")).toBeInTheDocument();
  });

  it("renders the chain mode and a hook badge per declared hook", async () => {
    chainMock.mockResolvedValue({
      mode: "stacked",
      chain_length: 2,
      fallback: "default engine (embedding recall)",
      chain: [
        {
          plugin: "memory-lens",
          hooks: coverage({ ingest: true, assemble: true, runtime: "python", cache_ttl_secs: 60 }),
        },
        { plugin: "broken-plugin", hooks: { error: "plugin manifest not found" } },
      ],
    });
    renderPage();

    const card = await screen.findByTestId("context-engine-chain");
    expect(within(card).getAllByText("context_engine.chain.mode_stacked")).toHaveLength(2);
    expect(within(card).getByText("memory-lens")).toBeInTheDocument();
    expect(within(card).getByText("ingest")).toBeInTheDocument();
    expect(within(card).getByText("assemble")).toBeInTheDocument();
    expect(within(card).getByText("python")).toBeInTheDocument();
    expect(within(card).getByText(/plugin manifest not found/)).toBeInTheDocument();
    expect(within(card).getByText("broken-plugin")).toBeInTheDocument();
  });

  it("renders per-hook metrics rows", async () => {
    metricsMock.mockResolvedValue(
      metricsWith({ ingest: { calls: 10, successes: 9, failures: 1, total_ms: 120 } }),
    );
    renderPage();

    const card = await screen.findByTestId("context-engine-metrics");
    const row = within(card).getByText("ingest").closest("tr");
    expect(row).not.toBeNull();
    expect(within(row!).getByText("10")).toBeInTheDocument();
    expect(within(row!).getByText("9")).toBeInTheDocument();
    expect(within(row!).getByText("1")).toBeInTheDocument();
    expect(within(row!).getByText("12 ms")).toBeInTheDocument();
    expect(within(card).getByText("after_turn")).toBeInTheDocument();
    expect(within(card).getByText("merge_subagent")).toBeInTheDocument();
  });

  it("renders the traces empty state", async () => {
    tracesMock.mockResolvedValue({ traces: [], count: 0 });
    renderPage();

    const card = await screen.findByTestId("context-engine-traces");
    expect(within(card).getByText("context_engine.traces.empty")).toBeInTheDocument();
    expect(within(card).getByText("context_engine.traces.empty_desc")).toBeInTheDocument();
  });

  it("renders traces newest first with success, duration and preview", async () => {
    const traces: ContextEngineTraces = {
      count: 2,
      traces: [
        {
          trace_id: "aaaaaaaaaaaaaaaa",
          correlation_id: "turn-1",
          hook: "assemble",
          started_at: "2026-01-01T10:00:00Z",
          elapsed_ms: 5,
          success: true,
          input_preview: { messages: 4 },
        },
        {
          trace_id: "bbbbbbbbbbbbbbbb",
          correlation_id: "turn-1",
          hook: "ingest",
          started_at: "2026-01-01T10:05:00Z",
          elapsed_ms: 9,
          success: false,
          error: "hook timed out",
          input_preview: { message: "hello" },
        },
      ],
    };
    tracesMock.mockResolvedValue(traces);
    renderPage();

    const card = await screen.findByTestId("context-engine-traces");
    const rows = within(card).getAllByTestId("context-engine-trace");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("ingest");
    expect(rows[0]).toHaveTextContent("9 ms");
    expect(rows[0]).toHaveTextContent("context_engine.traces.failure");
    expect(rows[0]).toHaveTextContent("hook timed out");
    expect(rows[0]).toHaveTextContent('{"message":"hello"}');
    expect(rows[1]).toHaveTextContent("assemble");
    expect(rows[1]).toHaveTextContent("context_engine.traces.success");
    expect(rows[1]).toHaveTextContent("5 ms");
  });

  it("renders the read-only configuration summary", async () => {
    configMock.mockResolvedValue({
      ...defaultConfig,
      engine: "scriptable",
      plugin: "policy-plugin",
      hook_timeout_secs: 45,
      on_hook_failure: "abort",
      max_retries: 2,
      retry_delay_ms: 250,
      hook_cache_ttl_secs: 60,
      allow_network: false,
      allow_filesystem: true,
      max_memory_mb: 256,
      circuit_breaker: { max_failures: 5, reset_secs: 30 },
      registries: [{ name: "Official", github_repo: "librefang/librefang-registry" }],
    });
    renderPage();

    const card = await screen.findByTestId("context-engine-config");
    expect(within(card).getByText("scriptable")).toBeInTheDocument();
    expect(within(card).getByText("policy-plugin")).toBeInTheDocument();
    expect(within(card).getByText("45s")).toBeInTheDocument();
    expect(within(card).getByText("abort")).toBeInTheDocument();
    expect(within(card).getByText("2")).toBeInTheDocument();
    expect(within(card).getByText("250 ms")).toBeInTheDocument();
    expect(within(card).getByText("60s")).toBeInTheDocument();
    expect(within(card).getByText("runtime.disabled")).toBeInTheDocument();
    expect(within(card).getByText("runtime.enabled")).toBeInTheDocument();
    expect(within(card).getByText("256 MB")).toBeInTheDocument();
    expect(within(card).getByText("5 / 30s")).toBeInTheDocument();
    expect(within(card).getByText("Official")).toBeInTheDocument();
  });

  it("renders one sandbox policy card per plugin", async () => {
    sandboxMock.mockResolvedValue({
      mode: "single",
      plugins: [
        {
          name: "memory-lens",
          allow_filesystem: true,
          allow_network: false,
          max_memory_mb: 128,
          otel_endpoint: "http://otel:4317",
          persistent_subprocess: true,
          prewarm_subprocesses: false,
          after_turn_queue_depth: 4,
          circuit_breaker: { max_failures: 3, reset_secs: 30 },
        },
        { name: "broken-plugin", error: "manifest not found" },
      ],
    });
    renderPage();

    const card = await screen.findByTestId("context-engine-sandbox");
    expect(within(card).getByText("memory-lens")).toBeInTheDocument();
    expect(within(card).getByText("broken-plugin")).toBeInTheDocument();
    expect(within(card).getByText("128 MB")).toBeInTheDocument();
    expect(within(card).getByText("http://otel:4317")).toBeInTheDocument();
    expect(within(card).getByText("3 / 30s")).toBeInTheDocument();
    expect(within(card).getByText(/manifest not found/)).toBeInTheDocument();
    expect(within(card).getAllByText("context_engine.sandbox.allowed").length).toBeGreaterThan(0);
    expect(within(card).getAllByText("context_engine.sandbox.denied").length).toBeGreaterThan(0);
  });

  it("shows the shared error state and retries every query", async () => {
    healthMock.mockRejectedValue(new Error("health down"));
    chainMock.mockRejectedValue(new Error("chain down"));
    metricsMock.mockRejectedValue(new Error("metrics down"));
    tracesMock.mockRejectedValue(new Error("traces down"));
    configMock.mockRejectedValue(new Error("config down"));
    sandboxMock.mockRejectedValue(new Error("sandbox down"));
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("context_engine.load_error")).toBeInTheDocument();
    expect(healthMock).toHaveBeenCalledTimes(1);

    fireEvent.click(within(alert).getByRole("button", { name: "common.refresh" }));

    await waitFor(() => expect(healthMock).toHaveBeenCalledTimes(2));
    expect(chainMock).toHaveBeenCalledTimes(2);
    expect(metricsMock).toHaveBeenCalledTimes(2);
    expect(tracesMock).toHaveBeenCalledTimes(2);
    expect(configMock).toHaveBeenCalledTimes(2);
    expect(sandboxMock).toHaveBeenCalledTimes(2);
  });
});
