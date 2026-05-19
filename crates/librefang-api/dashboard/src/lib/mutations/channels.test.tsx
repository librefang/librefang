import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import type { SidecarSaveResult } from "../../api";
import { useSaveSidecarConfig } from "./channels";
import { channelKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

// Mutations import from the typed http/client whitelist (see dashboard/CLAUDE.md),
// so the mock target is "../http/client", not "../../api".
vi.mock("../http/client", () => ({
  saveSidecarConfig: vi.fn(),
}));

const sidecarSaved: SidecarSaveResult = {
  status: "saved",
  restart_required: false,
  hot_actions_applied: ["ReloadChannels"],
};

describe("useSaveSidecarConfig", () => {
  beforeEach(() => {
    vi.mocked(http.saveSidecarConfig).mockResolvedValue(sidecarSaved);
  });

  it("calls saveSidecarConfig and invalidates channelKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSaveSidecarConfig(), { wrapper });

    result.current.mutate({
      name: "telegram",
      values: { TELEGRAM_BOT_TOKEN: "x" },
    });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(http.saveSidecarConfig).toHaveBeenCalledWith("telegram", {
      TELEGRAM_BOT_TOKEN: "x",
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: channelKeys.all,
    });
  });
});
