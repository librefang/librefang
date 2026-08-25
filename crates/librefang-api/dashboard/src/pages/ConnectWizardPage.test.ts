import { describe, expect, it } from "vitest";

import { decodeQrPayload } from "./ConnectWizardPage";

function qrFor(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
  return `librefang://pair?payload=${encoded}`;
}

describe("decodeQrPayload", () => {
  it("rejects an invalid expiry", () => {
    expect(() => decodeQrPayload(qrFor({
      v: 1,
      base_url: "https://daemon.example",
      token: "pairing-token",
      expires_at: "not-a-date",
    }))).toThrow("expired or has an invalid expiry");
  });

  it("reports damaged payloads with a pairing-specific error", () => {
    expect(() => decodeQrPayload("librefang://pair?payload=%%%"))
      .toThrow("Invalid QR code: could not decode payload");
  });
});
