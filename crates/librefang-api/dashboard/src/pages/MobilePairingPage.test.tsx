// Tests for the mobile pairing page (refs #3853 — pages/ test gap).
//
// Mocks at the queries/mutations hook layer per the dashboard data-layer rule:
// pages MUST go through `lib/queries`, never `fetch()`. Pairing is security-
// adjacent (QR code + device removal) so we cover happy/loading/error paths
// plus the device-removal mutation wiring.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MobilePairingPage } from "./MobilePairingPage";
import {
  usePairingRequest,
  usePairedDevices,
  useRemovePairedDevice,
} from "../lib/queries/pairing";
import { ApiError } from "../lib/http/errors";

vi.mock("../lib/queries/pairing", () => ({
  usePairingRequest: vi.fn(),
  usePairedDevices: vi.fn(),
  useRemovePairedDevice: vi.fn(),
}));

// qrcode pulls canvas drawing APIs jsdom only stubs. The page just delegates
// to it inside a useEffect; replacing it with a no-op keeps the component
// pure render testable.
vi.mock("qrcode", () => ({
  default: { toCanvas: vi.fn().mockResolvedValue(undefined) },
}));

vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");

  const translations: Record<string, string> = {
    "mobile_pairing.title": "Mobile Pairing",
    "mobile_pairing.subtitle":
      "Open the LibreFang mobile app and tap <strong>Scan QR</strong> to connect to this daemon.",
    "mobile_pairing.expired_message":
      "QR code expired — refresh to get a new one.",
    "mobile_pairing.refresh": "Refresh",
    "mobile_pairing.expired_label": "Expired",
    "mobile_pairing.paired_devices_heading": "Paired Devices",
    "mobile_pairing.paired_at": "{{platform}} · paired {{date}}",
    "mobile_pairing.remove_title": "Remove device",
    "mobile_pairing.remove_failed": "Failed to remove device: {{reason}}",
    "mobile_pairing.remove_unknown_error": "unknown error",
    "mobile_pairing.error_disabled_title": "Device pairing is disabled",
    "mobile_pairing.error_disabled_body":
      "Enable pairing in <link>Config → Security</link> (<code>pairing.enabled = true</code>).",
    "mobile_pairing.error_generic_title": "Failed to generate pairing code",
    "mobile_pairing.qr_aria_label":
      "QR code for pairing with a mobile device",
    "mobile_pairing.btn_try_again": "Try again",
  };

  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        let text = translations[key] ?? key;
        if (opts && typeof opts === "object") {
          for (const [k, v] of Object.entries(opts)) {
            text = text.replace(`{{${k}}}`, String(v));
          }
        }
        return text;
      },
    }),
  };
});

const usePairingRequestMock = usePairingRequest as unknown as ReturnType<
  typeof vi.fn
>;
const usePairedDevicesMock = usePairedDevices as unknown as ReturnType<
  typeof vi.fn
>;
const useRemovePairedDeviceMock = useRemovePairedDevice as unknown as ReturnType<
  typeof vi.fn
>;

function renderPage(): void {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <MobilePairingPage />
    </QueryClientProvider>,
  );
}

const FUTURE_ISO = new Date(Date.now() + 5 * 60 * 1000).toISOString();
const PAST_ISO = new Date(Date.now() - 60 * 1000).toISOString();

describe("MobilePairingPage", () => {
  let removeMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    removeMutate = vi.fn();
    useRemovePairedDeviceMock.mockReturnValue({
      mutate: removeMutate,
      isPending: false,
      isError: false,
      error: null,
    });
    usePairedDevicesMock.mockReturnValue({ data: [] });
  });

  it("renders the loading spinner while the pairing request is in flight", () => {
    usePairingRequestMock.mockReturnValue({
      data: undefined,
      error: null,
      isLoading: true,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("Mobile Pairing")).toBeInTheDocument();
    // No QR canvas yet, no refresh control either.
    expect(screen.queryByText("Refresh")).not.toBeInTheDocument();
  });

  it("renders the disabled-feature message when the API returns 404", () => {
    usePairingRequestMock.mockReturnValue({
      data: undefined,
      error: new ApiError(404, "NOT_FOUND", "Not found"),
      isLoading: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(
      screen.getByText("Device pairing is disabled"),
    ).toBeInTheDocument();
    // Disabled state shows a help link, not the retry button.
    expect(screen.queryByText("Try again")).not.toBeInTheDocument();
  });

  it("renders a generic error and a retry button on non-404 failures", () => {
    const refetch = vi.fn();
    usePairingRequestMock.mockReturnValue({
      data: undefined,
      error: new ApiError(500, "INTERNAL_ERROR", "Internal Server Error"),
      isLoading: false,
      refetch,
    });

    renderPage();

    expect(
      screen.getByText("Failed to generate pairing code"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Try again" }),
    );
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("renders the QR card and countdown when a fresh pairing token is returned", () => {
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=abc", expires_at: FUTURE_ISO },
      error: null,
      isLoading: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(screen.getByText("Refresh")).toBeInTheDocument();
    // Not expired, so no expiration banner.
    expect(
      screen.queryByText("QR code expired — refresh to get a new one."),
    ).not.toBeInTheDocument();
  });

  it("shows the expired message when the token's expires_at is already past", () => {
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=stale", expires_at: PAST_ISO },
      error: null,
      isLoading: false,
      refetch: vi.fn(),
    });

    renderPage();

    expect(
      screen.getByText("QR code expired — refresh to get a new one."),
    ).toBeInTheDocument();
  });

  it("calls refetch when the inline refresh button is clicked", () => {
    const refetch = vi.fn();
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=abc", expires_at: FUTURE_ISO },
      error: null,
      isLoading: false,
      refetch,
    });

    renderPage();

    fireEvent.click(
      screen.getByRole("button", { name: /Refresh/ }),
    );
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("lists paired devices and wires the trash button to useRemovePairedDevice", () => {
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=abc", expires_at: FUTURE_ISO },
      error: null,
      isLoading: false,
      refetch: vi.fn(),
    });
    usePairedDevicesMock.mockReturnValue({
      data: [
        {
          device_id: "dev-1",
          display_name: "Pixel 8",
          platform: "android",
          paired_at: new Date("2025-01-15T10:00:00Z").toISOString(),
        },
      ],
    });

    renderPage();

    expect(screen.getByText("Pixel 8")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Remove device" }),
    );
    expect(removeMutate).toHaveBeenCalledTimes(1);
    expect(removeMutate).toHaveBeenCalledWith("dev-1");
  });

  it("surfaces a remove-device error banner with the underlying reason", () => {
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=abc", expires_at: FUTURE_ISO },
      error: null,
      isLoading: false,
      refetch: vi.fn(),
    });
    usePairedDevicesMock.mockReturnValue({
      data: [
        {
          device_id: "dev-1",
          display_name: "Pixel 8",
          platform: "android",
          paired_at: new Date().toISOString(),
        },
      ],
    });
    useRemovePairedDeviceMock.mockReturnValue({
      mutate: removeMutate,
      isPending: false,
      isError: true,
      error: new Error("forbidden"),
    });

    renderPage();

    expect(screen.getByText(/forbidden/)).toBeInTheDocument();
  });

  it("hides the paired-devices section entirely when the list is empty", () => {
    usePairingRequestMock.mockReturnValue({
      data: { qr_uri: "librefang://pair?t=abc", expires_at: FUTURE_ISO },
      error: null,
      isLoading: false,
      refetch: vi.fn(),
    });
    usePairedDevicesMock.mockReturnValue({ data: [] });

    renderPage();

    expect(
      screen.queryByText("Paired Devices"),
    ).not.toBeInTheDocument();
  });
});
