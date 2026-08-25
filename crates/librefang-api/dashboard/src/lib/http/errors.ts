export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    Object.setPrototypeOf(this, ApiError.prototype);
  }

  static async fromResponse(response: Response): Promise<ApiError> {
    let message = response.statusText;
    let code = `HTTP_${response.status}`;
    let text = "";

    try {
      text = await response.text();
    } catch {
      return new ApiError(response.status, code, message || `HTTP ${response.status}`);
    }

    try {
      const json = JSON.parse(text) as Record<string, unknown>;
      // #3639 deferred: prefer the nested `error: {code, message, request_id}`
      // envelope when present; fall back to the legacy flat shape so we
      // keep parsing responses from older daemons during the rollout.
      //
      // Four supported shapes (checked in priority order):
      //   1. Nested envelope — { error: { code, message, request_id } }
      //   2. Flat detail     — { detail: "..." }
      //   3. Flat message    — { message: "..." }
      //   4. Flat error      — { error: "..." }
      const nested =
        typeof json.error === "object" && json.error !== null
          ? (json.error as Record<string, unknown>)
          : null;

      if (nested && typeof nested.message === "string") {
        message = nested.message;
      } else if (typeof json.detail === "string") {
        message = json.detail;
      } else if (typeof json.message === "string") {
        message = json.message;
      } else if (typeof json.error === "string") {
        message = json.error;
      }

      if (nested) {
        if (typeof nested.code === "string") code = nested.code;
      } else if (typeof json.code === "string") {
        code = json.code;
      }
    } catch {
      // ignore parse errors
    }

    return new ApiError(response.status, code, message || `HTTP ${response.status}`);
  }
}

/**
 * True when the daemon reached a skill marketplace but the marketplace did not answer with usable data.
 *
 * Two statuses mean the same thing to a reader of the Skills page.
 * `502` is what every Skillhub / ClawHub handler returns today: the hub answers `200` with its own web shell, `serde_json` refuses the leading `<`, and the failure surfaces as a generic upstream error whose message is the parser's complaint.
 * `503` is what the parsing boundary reports once it recognises "the marketplace is serving a webpage" as its own condition (#7748).
 *
 * Either way the request was fine and the hub is not — so the UI owes the reader an offline state, not a parser transcript.
 */
export function isMarketplaceUnavailable(err: unknown): boolean {
  if (!err || typeof err !== "object") return false;
  const status = (err as { status?: unknown }).status;
  return status === 502 || status === 503;
}
