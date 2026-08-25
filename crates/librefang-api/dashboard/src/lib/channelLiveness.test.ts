import { describe, it, expect, vi } from "vitest";
import { channelLiveness, livenessLabel, type ChannelLivenessState } from "./channelLiveness";
import type { ChannelItem } from "../api";

// State mapping in isolation. Kept separate from the render suites so a
// regression points at the rule rather than at either page's card markup.
describe("channelLiveness", () => {
  function ch(overrides: Partial<ChannelItem>): ChannelItem {
    return { name: "c", configured: true, ...overrides };
  }

  it("reports a neutral setup state for an unconfigured catalog row", () => {
    expect(channelLiveness(ch({ configured: false }))).toEqual({
      state: "unconfigured",
      variant: "default",
      error: null,
    });
  });

  it("reports not_supervised when no adapter is registered", () => {
    expect(channelLiveness(ch({ supervised: false, connected: false }))).toMatchObject({
      state: "not_supervised",
      variant: "warning",
    });
  });

  it("reports failed when down with a recorded error", () => {
    expect(
      channelLiveness(
        ch({ supervised: true, connected: false, last_error: "boom", started_at: null }),
      ),
    ).toMatchObject({ state: "failed", variant: "error", error: "boom" });
  });

  it("reports stopped when down after having been up, with no error", () => {
    expect(
      channelLiveness(
        ch({ supervised: true, connected: false, started_at: "2030-01-01T00:00:00Z" }),
      ),
    ).toMatchObject({ state: "stopped", variant: "error" });
  });

  it("reports starting when registered but never yet connected", () => {
    expect(
      channelLiveness(ch({ supervised: true, connected: false, started_at: null })),
    ).toMatchObject({ state: "starting", variant: "warning" });
  });

  it("reports degraded — not active — for a connected channel with a sticky error", () => {
    // `last_error` is never cleared by the supervisor, so a busy channel that
    // failed once still carries one. It must not read as healthy.
    expect(
      channelLiveness(
        ch({
          supervised: true,
          connected: true,
          messages_received: 99,
          last_error: "Sidecar adapter reported error",
        }),
      ),
    ).toMatchObject({ state: "degraded", variant: "warning" });
  });

  it("reports active when connected with traffic in either direction", () => {
    expect(
      channelLiveness(ch({ supervised: true, connected: true, messages_sent: 1 })),
    ).toMatchObject({ state: "active", variant: "success" });
    expect(
      channelLiveness(ch({ supervised: true, connected: true, messages_received: 1 })),
    ).toMatchObject({ state: "active", variant: "success" });
  });

  it("reports connected — still green — when connected with no traffic", () => {
    expect(
      channelLiveness(
        ch({ supervised: true, connected: true, messages_received: 0, messages_sent: 0 }),
      ),
    ).toMatchObject({ state: "connected", variant: "success" });
  });

  it("ignores the per-channel-type 24h figure entirely", () => {
    // The number a dead bot's live siblings produce must not affect it.
    expect(
      channelLiveness(
        ch({
          supervised: true,
          connected: false,
          started_at: "2030-01-01T00:00:00Z",
          msgs_24h_channel_type: 9999,
        }),
      ).state,
    ).toBe("stopped");
  });

  it("treats a blank last_error as no error", () => {
    expect(
      channelLiveness(ch({ supervised: true, connected: true, last_error: "   " })).error,
    ).toBeNull();
  });

  it("fails visibly when last_error drifts to a non-string payload", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const channel = ch({
      supervised: true,
      connected: false,
      last_error: { message: "boom" } as unknown as string,
    });

    expect(channelLiveness(channel)).toMatchObject({
      state: "failed",
      variant: "error",
      error: "Invalid last_error payload",
    });
    expect(warn).toHaveBeenCalledOnce();
    warn.mockRestore();
  });
});

describe("livenessLabel", () => {
  const states: ChannelLivenessState[] = [
    "unconfigured",
    "not_supervised",
    "failed",
    "stopped",
    "starting",
    "degraded",
    "active",
    "connected",
  ];

  it("returns a distinct channels.liveness.* key for every state", () => {
    // Guards the exhaustive switch: adding a state without a label would fall
    // through and return undefined, which renders as an empty badge — the
    // colour-only indicator #6606 exists to remove.
    const keys = states.map((s) => livenessLabel(s, (k) => k));
    expect(keys).toEqual([
      "common.setup",
      "channels.liveness.not_supervised",
      "channels.liveness.failed",
      "channels.liveness.stopped",
      "channels.liveness.starting",
      "channels.liveness.degraded",
      "channels.liveness.active",
      "channels.liveness.connected",
    ]);
    expect(new Set(keys).size).toBe(states.length);
  });

  it("falls back to an unknown runtime state instead of returning undefined", () => {
    expect(livenessLabel("future" as ChannelLivenessState, (key) => key)).toBe("future");
  });
});
