import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { pluginKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useScaffoldPlugin } from "./plugins";

vi.mock("../http/client", () => ({
  scaffoldPlugin: vi.fn().mockResolvedValue({ status: "ok" }),
}));

describe("useScaffoldPlugin", () => {
  it("maps the named description to the scaffold API and invalidates plugin views", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useScaffoldPlugin(), { wrapper });

    await result.current.mutateAsync({
      name: "example-plugin",
      description: "Example description",
      runtime: "python",
    });

    expect(http.scaffoldPlugin).toHaveBeenCalledWith(
      "example-plugin",
      "Example description",
      "python",
    );
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: pluginKeys.all });
  });
});
