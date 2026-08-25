import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { terminalKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDeleteTerminalWindow, useRenameTerminalWindow } from "./terminal";

vi.mock("../http/client", () => ({
  createTerminalWindow: vi.fn(),
  renameTerminalWindow: vi.fn(),
  deleteTerminalWindow: vi.fn(),
}));

const windows = [
  { id: "one", index: 0, name: "One", active: true },
  { id: "two", index: 1, name: "Two", active: false },
];

describe("terminal window mutations", () => {
  beforeEach(() => {
    vi.mocked(http.renameTerminalWindow).mockReset().mockResolvedValue(undefined);
    vi.mocked(http.deleteTerminalWindow).mockReset().mockResolvedValue(undefined);
  });

  it("optimistically renames and reconciles only the windows cache", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(terminalKeys.windows(), windows);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRenameTerminalWindow(), { wrapper });

    await result.current.mutateAsync({ windowId: "two", name: "Renamed" });

    expect(queryClient.getQueryData(terminalKeys.windows())).toEqual([
      windows[0],
      { ...windows[1], name: "Renamed" },
    ]);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: terminalKeys.windows() });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: terminalKeys.all });
  });

  it("rolls a failed rename back and still reconciles", async () => {
    vi.mocked(http.renameTerminalWindow).mockRejectedValueOnce(new Error("rename failed"));
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(terminalKeys.windows(), windows);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useRenameTerminalWindow(), { wrapper });

    await expect(result.current.mutateAsync({ windowId: "two", name: "Renamed" }))
      .rejects.toThrow("rename failed");

    expect(queryClient.getQueryData(terminalKeys.windows())).toEqual(windows);
    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: terminalKeys.windows() });
    });
  });

  it("optimistically removes a deleted window and reconciles", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(terminalKeys.windows(), windows);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteTerminalWindow(), { wrapper });

    await result.current.mutateAsync("one");

    expect(queryClient.getQueryData(terminalKeys.windows())).toEqual([windows[1]]);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: terminalKeys.windows() });
  });

  it("rolls a failed delete back and still reconciles", async () => {
    vi.mocked(http.deleteTerminalWindow).mockRejectedValueOnce(new Error("delete failed"));
    const { queryClient, wrapper } = createQueryClientWrapper();
    queryClient.setQueryData(terminalKeys.windows(), windows);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDeleteTerminalWindow(), { wrapper });

    await expect(result.current.mutateAsync("one")).rejects.toThrow("delete failed");

    expect(queryClient.getQueryData(terminalKeys.windows())).toEqual(windows);
    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: terminalKeys.windows() });
    });
  });
});
