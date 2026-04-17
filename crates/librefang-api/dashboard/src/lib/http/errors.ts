export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }

  static async fromResponse(response: Response): Promise<ApiError> {
    const text = await response.text();
    let message = response.statusText;
    let code = `HTTP_${response.status}`;

    try {
      const json = JSON.parse(text) as Record<string, unknown>;
      if (typeof json.detail === "string") {
        message = json.detail;
      } else if (typeof json.error === "string") {
        message = json.error;
        code = json.error;
      }
    } catch {
      // ignore parse errors
    }

    return new ApiError(response.status, code, message || `HTTP ${response.status}`);
  }
}
