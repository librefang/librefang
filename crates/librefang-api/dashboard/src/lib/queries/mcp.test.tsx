import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  getMcpAuthStatus,
  getMcpCatalogEntry,
  getMcpServer,
} from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import {
  mcpQueries,
  useMcpAuthStatus,
  useMcpCatalogEntry,
  useMcpServer,
} from "./mcp";

vi.mock("../http/client", () => ({
  listMcpServers: vi.fn(),
  getMcpServer: vi.fn(),
  listMcpCatalog: vi.fn(),
  getMcpCatalogEntry: vi.fn(),
  getMcpHealth: vi.fn(),
  getMcpAuthStatus: vi.fn(),
  listMcpTaintRules: vi.fn(),
}));

describe("MCP query contracts", () => {
  it("keeps the catalog factory enabled by default", () => {
    expect(mcpQueries.catalog().enabled).toBeUndefined();
  });

  it("names the short OAuth status freshness window", () => {
    expect(mcpQueries.authStatus("server-1").staleTime).toBe(2_000);
  });

  it("cannot enable ID-scoped reads without an ID", () => {
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useMcpServer("", { enabled: true }), { wrapper });
    renderHook(() => useMcpCatalogEntry("", { enabled: true }), { wrapper });
    renderHook(() => useMcpAuthStatus("", { enabled: true }), { wrapper });

    expect(getMcpServer).not.toHaveBeenCalled();
    expect(getMcpCatalogEntry).not.toHaveBeenCalled();
    expect(getMcpAuthStatus).not.toHaveBeenCalled();
  });
});
