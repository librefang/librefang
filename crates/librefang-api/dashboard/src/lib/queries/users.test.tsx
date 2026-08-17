import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as httpClient from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { useUsers } from "./users";

vi.mock("../http/client", () => ({
  listUsers: vi.fn(),
  getUser: vi.fn(),
}));

describe("useUsers", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shares one raw-list request across role and search views", async () => {
    vi.mocked(httpClient.listUsers).mockResolvedValue([
      {
        name: "Alice",
        role: "Admin",
        channel_bindings: { telegram: "OpsRoom" },
        has_api_key: true,
        has_policy: false,
        has_memory_access: false,
        has_budget: false,
      },
      {
        name: "Bob",
        role: "viewer",
        channel_bindings: {},
        has_api_key: false,
        has_policy: false,
        has_memory_access: false,
        has_budget: false,
      },
    ]);
    const { wrapper } = createQueryClientWrapper();
    const admin = renderHook(() => useUsers({ role: "admin" }), { wrapper });
    const channel = renderHook(() => useUsers({ search: "opsroom" }), { wrapper });

    await waitFor(() => expect(admin.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(channel.result.current.isSuccess).toBe(true));

    expect(admin.result.current.data?.map((user) => user.name)).toEqual(["Alice"]);
    expect(channel.result.current.data?.map((user) => user.name)).toEqual(["Alice"]);
    expect(httpClient.listUsers).toHaveBeenCalledTimes(1);
  });

  it("ignores malformed channel-binding values instead of failing the query", async () => {
    vi.mocked(httpClient.listUsers).mockResolvedValue([
      {
        name: "Alice",
        role: "admin",
        channel_bindings: { telegram: 123, slack: "team-alpha" },
      },
      { name: "Bob", role: "viewer", channel_bindings: undefined },
    ] as unknown as Awaited<ReturnType<typeof httpClient.listUsers>>);
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useUsers({ search: "team-alpha" }), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.map((user) => user.name)).toEqual(["Alice"]);
  });
});
