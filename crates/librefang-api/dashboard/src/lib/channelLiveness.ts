import type { ChannelItem } from "../api";
import type { BadgeVariant } from "../components/ui/Badge";

/** Minimal `useTranslation().t` shape: a key plus optional interpolation args. */
export type TFunc = (key: string, opts?: Record<string, unknown>) => string;

/**
 * Health of one sidecar channel instance, as the supervisor reports it.
 *
 * Ordered from "no adapter at all" to "healthy and busy".
 * Every state is derived from a fact the backend can substantiate — see `sidecar_channel_rows` in `crates/librefang-api/src/routes/channels.rs`.
 */
export type ChannelLivenessState =
  | "unconfigured"
  | "not_supervised"
  | "failed"
  | "stopped"
  | "starting"
  | "degraded"
  | "active"
  | "connected";

export interface ChannelLiveness {
  state: ChannelLivenessState;
  variant: BadgeVariant;
  /** Sticky supervisor error text, or null when none was recorded. */
  error: string | null;
}

/**
 * Map the supervisor's liveness fields onto a channel's status indicator.
 *
 * This replaces the pre-#6606 rule `msgs_24h > 0 ? "running" : "idle"`, which was a traffic proxy rather than health *and* read a per-channel-type aggregate, so on a host with six Telegram bots every card turned green as soon as any one of them saw traffic — and a bot that died after receiving a message stayed green.
 * Neither `connected` nor `last_error` was even present on the payload.
 *
 * It lives here rather than on a page because two pages render the same payload: the Channels page's cards and details drawer, and the Comms page's Channels tab.
 * A second copy of the mapping is how the Comms page came to paint a dead bot green from `configured` alone while the Channels page reported it correctly.
 *
 * The mapping, in evaluation order, and why each state has the colour it does:
 *
 * 1. `unconfigured` — neutral.
 *    The catalog knows the adapter, but the operator has not configured it.
 * 2. `not_supervised` — amber.
 *    Configured in `config.toml` but no adapter is registered: the sidecar was never started, or its start failed and the registration was rolled back.
 *    Amber rather than grey deliberately — grey reads as "fine, just quiet", which is exactly the misreading this fix exists to remove.
 * 3. `failed` — red.
 *    Down, and the supervisor recorded why.
 * 4. `stopped` — red.
 *    Down after having been up (`started_at` is set) with no error recorded — a silent exit.
 * 5. `starting` — amber.
 *    Registered, never yet connected, nothing wrong reported.
 *    The transient window between registration and first spawn.
 * 6. `degraded` — amber.
 *    Connected, but carrying an error.
 *    `last_error` is sticky (the supervisor never clears it, not even on the successful respawn that follows), so this means "failed at least once since the adapter was created", not "broken right now".
 *    Amber, not red — but it must not read as healthy either.
 * 7. `active` — green.
 *    Connected with messages in either direction.
 * 8. `connected` — green.
 *    Connected, no traffic yet.
 *    Distinguished from `active` by label, not colour: a quiet channel is healthy, and colouring it differently is what made a healthy-but-quiet bot look dead.
 *
 * Traffic here is the supervisor's own per-instance counters, which are since-adapter-creation.
 * The 24h figure on the payload is per channel type and deliberately plays no part in this mapping.
 *
 * Unconfigured catalog rows are classified explicitly so every caller gets a
 * neutral setup state without needing to duplicate a guard.
 */
export function channelLiveness(c: ChannelItem): ChannelLiveness {
  if (c.configured === false) {
    return { state: "unconfigured", variant: "default", error: null };
  }

  let error: string | null = null;
  if (typeof c.last_error === "string") {
    error = c.last_error.trim() || null;
  } else if (c.last_error != null) {
    console.warn("Channel last_error has an invalid non-string payload", c.last_error);
    error = "Invalid last_error payload";
  }
  if (!c.supervised) {
    return { state: "not_supervised", variant: "warning", error };
  }
  if (!c.connected) {
    if (error) return { state: "failed", variant: "error", error };
    if (c.started_at) return { state: "stopped", variant: "error", error };
    return { state: "starting", variant: "warning", error };
  }
  if (error) return { state: "degraded", variant: "warning", error };
  const traffic = (c.messages_received ?? 0) + (c.messages_sent ?? 0);
  return traffic > 0
    ? { state: "active", variant: "success", error }
    : { state: "connected", variant: "success", error };
}

/**
 * Short operator-facing label for a liveness state.
 *
 * Keys are spelled out literally so the locale-coverage check can see each one.
 */
export function livenessLabel(state: ChannelLivenessState, t: TFunc): string {
  switch (state) {
    case "unconfigured":
      return t("common.setup", { defaultValue: "Setup" });
    case "active":
      return t("channels.liveness.active", { defaultValue: "Active" });
    case "connected":
      return t("channels.liveness.connected", { defaultValue: "Connected" });
    case "degraded":
      return t("channels.liveness.degraded", { defaultValue: "Degraded" });
    case "starting":
      return t("channels.liveness.starting", { defaultValue: "Starting" });
    case "stopped":
      return t("channels.liveness.stopped", { defaultValue: "Stopped" });
    case "failed":
      return t("channels.liveness.failed", { defaultValue: "Failed" });
    case "not_supervised":
      return t("channels.liveness.not_supervised", {
        defaultValue: "Not started",
      });
    default:
      return state;
  }
}
