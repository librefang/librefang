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
