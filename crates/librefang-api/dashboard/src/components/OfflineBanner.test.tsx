import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OfflineBanner } from "./OfflineBanner";

const { healthRefetch, runtimeRefetch } = vi.hoisted(() => ({
  healthRefetch: vi.fn(),
  runtimeRefetch: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ refetchQueries: runtimeRefetch }),
}));

vi.mock("../lib/queries/runtime", () => ({
  useHealthLiveness: () => ({
    isError: true,
    isFetching: false,
    refetch: healthRefetch,
  }),
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

beforeEach(() => {
  healthRefetch.mockReset();
  runtimeRefetch.mockReset();
  runtimeRefetch.mockResolvedValue(undefined);
});

describe("OfflineBanner", () => {
  it("skips broad runtime refetch when liveness still fails", async () => {
    healthRefetch.mockResolvedValue({ isError: true });
    const user = userEvent.setup();
    render(<OfflineBanner />);

    await user.click(screen.getByRole("button", { name: "offline_banner.retry" }));

    await waitFor(() => expect(healthRefetch).toHaveBeenCalledTimes(1));
    expect(runtimeRefetch).not.toHaveBeenCalled();
  });

  it("ends retry UI before the background runtime refresh settles", async () => {
    healthRefetch.mockResolvedValue({ isError: false });
    runtimeRefetch.mockReturnValue(new Promise(() => {}));
    const user = userEvent.setup();
    render(<OfflineBanner />);
    const retryButton = screen.getByRole("button", { name: "offline_banner.retry" });

    await user.click(retryButton);

    await waitFor(() => expect(runtimeRefetch).toHaveBeenCalledTimes(1));
    expect(retryButton).not.toBeDisabled();
    expect(retryButton.querySelector("svg")).not.toHaveClass("animate-spin");
  });
});
