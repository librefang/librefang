import { describe, it, expect, vi } from "vitest";
import * as http from "../http/client";
import { renderHook } from "@testing-library/react";
import type { HandMessageResponse } from "../../api";
import {
  useActivateHand,
  useDeactivateHand,
  usePauseHand,
  useResumeHand,
  useUninstallHand,
  useSetHandSecret,
  useUpdateHandSettings,
  useSendHandMessage,
} from "./hands";
import { agentKeys, handKeys, overviewKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

type SetHandSecretInput = Parameters<ReturnType<typeof useSetHandSecret>["mutateAsync"]>[0];
type UpdateHandSettingsInput = Parameters<ReturnType<typeof useUpdateHandSettings>["mutateAsync"]>[0];

vi.mock("../http/client", () => ({
  activateHand: vi.fn(() => Promise.resolve({})),
  deactivateHand: vi.fn(() => Promise.resolve({})),
  pauseHand: vi.fn(() => Promise.resolve({})),
  resumeHand: vi.fn(() => Promise.resolve({})),
  uninstallHand: vi.fn(() => Promise.resolve({})),
  setHandSecret: vi.fn(() => Promise.resolve({})),
  updateHandSettings: vi.fn(() => Promise.resolve({})),
  sendHandMessage: vi.fn(() => Promise.resolve({})),
}));

describe("useActivateHand", () => {
  it("invalidates handKeys.all, agentKeys.all, and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useActivateHand(), { wrapper });

    await result.current.mutateAsync("hand-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useDeactivateHand", () => {
  it("invalidates handKeys.all, agentKeys.all, and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeactivateHand(), { wrapper });

    await result.current.mutateAsync("hand-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("usePauseHand", () => {
  it("invalidates handKeys.all, agentKeys.all, and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => usePauseHand(), { wrapper });

    await result.current.mutateAsync("hand-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useResumeHand", () => {
  it("invalidates handKeys.all, agentKeys.all, and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useResumeHand(), { wrapper });

    await result.current.mutateAsync("hand-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useUninstallHand", () => {
  it("invalidates handKeys.all, agentKeys.all, and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUninstallHand(), { wrapper });

    await result.current.mutateAsync("hand-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: agentKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useSetHandSecret", () => {
  it("invalidates handKeys.lists() and handKeys.detail(handId)", async () => {
    const args: SetHandSecretInput = { handId: "h1", key: "k", value: "v" };
    vi.mocked(http.setHandSecret).mockResolvedValue({ ok: true });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetHandSecret(), { wrapper });

    await result.current.mutateAsync(args);

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.detail("h1"),
    });
  });
});

describe("useUpdateHandSettings", () => {
  it("invalidates handKeys.lists() and handKeys.detail(handId)", async () => {
    const args: UpdateHandSettingsInput = { handId: "h1", config: { foo: 1 } };
    vi.mocked(http.updateHandSettings).mockResolvedValue({
      status: "ok",
      hand_id: "h1",
      instance_id: "inst-1",
      config: { foo: 1 },
    });
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateHandSettings(), { wrapper });

    await result.current.mutateAsync(args);

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.lists(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.detail("h1"),
    });
  });
});

describe("useSendHandMessage", () => {
  it("invalidates the session query and calls mutationFn with correct args", async () => {
    const response: HandMessageResponse = { response: "ok" };
    vi.mocked(http.sendHandMessage).mockResolvedValue(response);
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSendHandMessage(), { wrapper });

    await result.current.mutateAsync({ instanceId: "inst-1", message: "hello" });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: handKeys.session("inst-1"),
    });
    expect(http.sendHandMessage).toHaveBeenCalledWith("inst-1", "hello");
  });
});
