import React from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NetworkPage } from "./NetworkPage";
import {
  useNetworkStatus,
  usePeers,
  useTrustedPeers,
} from "../lib/queries/network";
import { useUIStore } from "../lib/store";

vi.mock("../lib/queries/network", () => ({
  useNetworkStatus: vi.fn(),
  usePeers: vi.fn(),
  useTrustedPeers: vi.fn(),
}));

vi.mock("../lib/store", () => ({
  useUIStore: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  const t = (key: string) => key;
  return {
    ...actual,
    useTranslation: () => ({ t }),
  };
});

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

const mock = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;
const useNetworkStatusMock = mock(useNetworkStatus);
const usePeersMock = mock(usePeers);
const useTrustedPeersMock = mock(useTrustedPeers);
const useUIStoreMock = mock(useUIStore);

const addToast = vi.fn();
const refetchStatus = vi.fn().mockResolvedValue(undefined);
const refetchPeers = vi.fn().mockResolvedValue(undefined);
const refetchTrusted = vi.fn().mockResolvedValue(undefined);

function query<T>(data: T, overrides: Record<string, unknown> = {}) {
  return {
    data,
    error: null,
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function setQueries(
  status: Record<string, unknown> = {
    online: true,
    node_id: "node-1",
    protocol_version: "1",
    identity_fingerprint: "fingerprint-123",
    pinned_peers: 0,
  },
) {
  useNetworkStatusMock.mockReturnValue(
    query(status, { refetch: refetchStatus }),
  );
  usePeersMock.mockReturnValue(query([], { refetch: refetchPeers }));
  useTrustedPeersMock.mockReturnValue(
    query([], { refetch: refetchTrusted }),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  setQueries();
  useUIStoreMock.mockImplementation(
    (selector: (state: { addToast: typeof addToast }) => unknown) =>
      selector({ addToast }),
  );
});

describe("NetworkPage", () => {
  it("renders an error state instead of empty success content when any query fails", () => {
    usePeersMock.mockReturnValue(
      query(undefined, {
        error: new Error("network unavailable"),
        isError: true,
        refetch: refetchPeers,
      }),
    );

    render(<NetworkPage />);

    expect(screen.getByRole("alert")).toHaveTextContent("common.error");
    expect(screen.queryByText("network.no_peers")).not.toBeInTheDocument();
    expect(screen.queryByText("network.no_trusted_peers")).not.toBeInTheDocument();
  });

  it("retries all three network queries from the error state", () => {
    useNetworkStatusMock.mockReturnValue(
      query(undefined, {
        error: new Error("network unavailable"),
        isError: true,
        refetch: refetchStatus,
      }),
    );
    render(<NetworkPage />);

    fireEvent.click(
      within(screen.getByRole("alert")).getByRole("button", {
        name: "common.refresh",
      }),
    );

    expect(refetchStatus).toHaveBeenCalledTimes(1);
    expect(refetchPeers).toHaveBeenCalledTimes(1);
    expect(refetchTrusted).toHaveBeenCalledTimes(1);
  });

  it("renders a fingerprint when the online node has one", () => {
    render(<NetworkPage />);

    expect(screen.getByText("fingerprint-123")).toBeInTheDocument();
    expect(screen.queryByText("network.identity_missing")).not.toBeInTheDocument();
  });

  it("renders the missing-identity warning for an online node", () => {
    setQueries({ online: true, identity_fingerprint: null, pinned_peers: 0 });
    render(<NetworkPage />);

    expect(screen.getByText("network.identity_missing")).toBeInTheDocument();
    expect(screen.queryByText("network.ofp_disabled")).not.toBeInTheDocument();
  });

  it("renders the disabled message for an offline node", () => {
    setQueries({ online: false, identity_fingerprint: null, pinned_peers: 0 });
    render(<NetworkPage />);

    expect(screen.getByText("network.ofp_disabled")).toBeInTheDocument();
    expect(screen.queryByText("network.identity_missing")).not.toBeInTheDocument();
  });
});
