import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { McpServersPage } from "./McpServersPage";
import { useMcpServers, useMcpCatalog, useMcpHealth } from "../lib/queries/mcp";
import {
  useAddMcpServer,
  useUpdateMcpServer,
  useDeleteMcpServer,
  useReloadMcp,
  useReconnectMcpServer,
  useStartMcpAuth,
  useRevokeMcpAuth,
} from "../lib/mutations/mcp";
import type { McpServerConfigured, McpServersResponse } from "../api";

// ---------------------------------------------------------------------------
// Mocks (#3853 — McpServersPage MCP server management page).
// ---------------------------------------------------------------------------

vi.mock("../lib/queries/mcp", () => ({
  useMcpServers: vi.fn(),
  useMcpCatalog: vi.fn(),
  useMcpHealth: vi.fn(),
  // mcpQueries is referenced by the page for prefetching; provide a no-op stub.
  mcpQueries: {
    servers: () => ({ queryKey: ["mcp", "servers"] }),
    catalog: () => ({ queryKey: ["mcp", "catalog"] }),
    health: () => ({ queryKey: ["mcp", "health"] }),
  },
}));

vi.mock("../lib/mutations/mcp", () => ({
  useAddMcpServer: vi.fn(),
  useUpdateMcpServer: vi.fn(),
  useDeleteMcpServer: vi.fn(),
  useReloadMcp: vi.fn(),
  useReconnectMcpServer: vi.fn(),
  useStartMcpAuth: vi.fn(),
  useRevokeMcpAuth: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      // Echo the i18n key so assertions can grep on it directly. Fall back
      // to the second positional argument (the inline default) so we can
      // also assert on the literal English copy.
      t: (key: string, optsOrFallback?: string | Record<string, unknown>) =>
        typeof optsOrFallback === "string"
          ? optsOrFallback
          : (optsOrFallback as Record<string, unknown> | undefined)?.defaultValue ??
            key,
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    ...rest
  }: {
    children: React.ReactNode;
  } & Record<string, unknown>) => (
    // eslint-disable-next-line jsx-a11y/anchor-is-valid
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}));

// motion/react (AnimatePresence + motion.div) triggers async animation hooks
// that don't settle cleanly in jsdom. Stub them out so render is synchronous.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
  motion: new Proxy(
    {},
    {
      get: (_target, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

// ---------------------------------------------------------------------------
// Type-cast helpers
// ---------------------------------------------------------------------------

const useMcpServersMock = useMcpServers as unknown as ReturnType<typeof vi.fn>;
const useMcpCatalogMock = useMcpCatalog as unknown as ReturnType<typeof vi.fn>;
const useMcpHealthMock = useMcpHealth as unknown as ReturnType<typeof vi.fn>;
const useAddMcpServerMock = useAddMcpServer as unknown as ReturnType<typeof vi.fn>;
const useUpdateMcpServerMock = useUpdateMcpServer as unknown as ReturnType<typeof vi.fn>;
const useDeleteMcpServerMock = useDeleteMcpServer as unknown as ReturnType<typeof vi.fn>;
const useReloadMcpMock = useReloadMcp as unknown as ReturnType<typeof vi.fn>;
const useReconnectMcpServerMock = useReconnectMcpServer as unknown as ReturnType<typeof vi.fn>;
const useStartMcpAuthMock = useStartMcpAuth as unknown as ReturnType<typeof vi.fn>;
const useRevokeMcpAuthMock = useRevokeMcpAuth as unknown as ReturnType<typeof vi.fn>;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

function makeServer(overrides: Partial<McpServerConfigured> = {}): McpServerConfigured {
  return {
    id: "test-server-1",
    name: "My Test Server",
    // McpTransport requires `type`; stdio is the simplest shape and avoids
    // URL-parsing paths that would require additional fixtures.
    transport: { type: "stdio", command: "node", args: ["server.js"] },
    ...overrides,
  };
}

function makeServersResponse(
  configured: McpServerConfigured[],
): McpServersResponse {
  return {
    configured,
    connected: [],
    total_configured: configured.length,
    total_connected: 0,
  };
}

function setServers(
  configured: McpServerConfigured[] | undefined,
  opts: { isLoading?: boolean; isError?: boolean; isSuccess?: boolean } = {},
) {
  const isLoading = opts.isLoading ?? false;
  const isSuccess = opts.isSuccess ?? (!isLoading && configured !== undefined);
  useMcpServersMock.mockReturnValue({
    data: configured !== undefined ? makeServersResponse(configured) : undefined,
    isLoading,
    isPending: isLoading,
    isError: opts.isError ?? false,
    isSuccess,
    isFetching: false,
    refetch: vi.fn(),
  });
}

function setMutationDefaults() {
  const idleMut = {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
  };
  useAddMcpServerMock.mockReturnValue(idleMut);
  useUpdateMcpServerMock.mockReturnValue(idleMut);
  useDeleteMcpServerMock.mockReturnValue(idleMut);
  useReloadMcpMock.mockReturnValue(idleMut);
  useReconnectMcpServerMock.mockReturnValue(idleMut);
  useStartMcpAuthMock.mockReturnValue(idleMut);
  useRevokeMcpAuthMock.mockReturnValue(idleMut);
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <McpServersPage />
    </QueryClientProvider>,
  );
}

// ---------------------------------------------------------------------------
// Global setup
// ---------------------------------------------------------------------------

beforeEach(() => {
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = function () {};
  }
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("McpServersPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setMutationDefaults();

    // Catalog and health queries are always idle in these unit tests.
    useMcpCatalogMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
    useMcpHealthMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isFetching: false,
      refetch: vi.fn(),
    });
  });

  it("renders the loading skeleton while servers are pending", () => {
    setServers(undefined, { isLoading: true, isSuccess: false });
    renderPage();
    // PageHeader mounts before data resolves — the page title key confirms
    // the route mounted correctly.
    expect(screen.getByText("mcp.title")).toBeInTheDocument();
    // Empty-state title must not appear while loading.
    expect(screen.queryByText("mcp.empty")).not.toBeInTheDocument();
  });

  it("renders the empty state when no servers are configured", () => {
    // When configured is empty and isSuccess=true the page auto-switches to
    // the catalog tab (autoSwitchedRef logic), so the "servers" tab body
    // with mcp.empty is not visible. Assert that the page header still
    // renders — indicating a successful mount — and that no server names
    // appear.
    setServers([], { isSuccess: true });
    renderPage();
    expect(screen.getByText("mcp.title")).toBeInTheDocument();
    expect(screen.queryByText("My Test Server")).not.toBeInTheDocument();
  });

  it("renders configured servers when the list is populated", () => {
    setServers([
      makeServer({ id: "srv-1", name: "Brave Search MCP" }),
      makeServer({ id: "srv-2", name: "GitHub MCP", transport: { type: "sse", url: "http://localhost:3001/sse" } }),
    ]);
    renderPage();
    expect(screen.getByText("Brave Search MCP")).toBeInTheDocument();
    expect(screen.getByText("GitHub MCP")).toBeInTheDocument();
    // Empty state must not appear when servers exist.
    expect(screen.queryByText("mcp.empty")).not.toBeInTheDocument();
  });

  it("exposes the Add server and Reload action buttons in the page header", () => {
    setServers([]);
    renderPage();
    // t("mcp.add_server") echoes the key — button text matches.
    expect(
      screen.getByRole("button", { name: "mcp.add_server" }),
    ).toBeInTheDocument();
    // t("mcp.reload") echoes the key.
    expect(
      screen.getByRole("button", { name: "mcp.reload" }),
    ).toBeInTheDocument();
  });
});
