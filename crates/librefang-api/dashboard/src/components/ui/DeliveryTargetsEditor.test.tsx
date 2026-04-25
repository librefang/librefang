import { describe, expect, it } from "vitest";
import { buildTarget, type DraftState } from "./DeliveryTargetsEditor";

const draft = (overrides: Partial<DraftState>): DraftState => ({
  type: "channel",
  channel_type: "telegram",
  recipient: "",
  thread_id: "",
  account_id: "",
  url: "",
  auth_header: "",
  path: "",
  append: true,
  to: "",
  subject_template: "",
  ...overrides,
});

describe("buildTarget — channel", () => {
  it("rejects missing channel_type", () => {
    const [t, err] = buildTarget(draft({ channel_type: "  ", recipient: "abc" }));
    expect(t).toBeNull();
    expect(err).toBe("scheduler.delivery.err_channel_type_required");
  });

  it("rejects missing recipient", () => {
    const [t, err] = buildTarget(draft({ recipient: "  " }));
    expect(t).toBeNull();
    expect(err).toBe("scheduler.delivery.err_recipient_required");
  });

  it("strips empty optional fields so they don't ship as Some(\"\")", () => {
    const [t, err] = buildTarget(
      draft({ recipient: "C123", thread_id: "  ", account_id: "" })
    );
    expect(err).toBeNull();
    expect(t).toEqual({ type: "channel", channel_type: "telegram", recipient: "C123" });
    expect(t).not.toHaveProperty("thread_id");
    expect(t).not.toHaveProperty("account_id");
  });

  it("includes optional fields when provided", () => {
    const [t] = buildTarget(
      draft({ recipient: "C123", thread_id: "1.2", account_id: "ws-b" })
    );
    expect(t).toEqual({
      type: "channel",
      channel_type: "telegram",
      recipient: "C123",
      thread_id: "1.2",
      account_id: "ws-b",
    });
  });
});

describe("buildTarget — webhook", () => {
  it("rejects missing url", () => {
    const [, err] = buildTarget(draft({ type: "webhook", url: "" }));
    expect(err).toBe("scheduler.delivery.err_url_required");
  });

  it("rejects non-http(s) scheme", () => {
    const [, err] = buildTarget(draft({ type: "webhook", url: "ftp://x.com" }));
    expect(err).toBe("scheduler.delivery.err_url_scheme");
  });

  it("rejects localhost (SSRF)", () => {
    const [, err] = buildTarget(draft({ type: "webhook", url: "http://localhost:8080/h" }));
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it("rejects loopback IPv4 (SSRF)", () => {
    const [, err] = buildTarget(draft({ type: "webhook", url: "http://127.0.0.1/h" }));
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it("rejects link-local / cloud metadata 169.254.169.254 (SSRF)", () => {
    const [, err] = buildTarget(
      draft({ type: "webhook", url: "http://169.254.169.254/latest/meta-data/" })
    );
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it("rejects metadata.google.internal (SSRF)", () => {
    const [, err] = buildTarget(
      draft({ type: "webhook", url: "http://metadata.google.internal/" })
    );
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it("rejects IPv6 loopback (SSRF)", () => {
    const [, err] = buildTarget(draft({ type: "webhook", url: "http://[::1]:8080/h" }));
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it("accepts a normal external host", () => {
    const [t, err] = buildTarget(draft({ type: "webhook", url: "https://example.com/hook" }));
    expect(err).toBeNull();
    expect(t).toEqual({ type: "webhook", url: "https://example.com/hook" });
  });

  it("strips empty auth_header", () => {
    const [t] = buildTarget(
      draft({ type: "webhook", url: "https://example.com/hook", auth_header: "  " })
    );
    expect(t).not.toHaveProperty("auth_header");
  });
});

describe("buildTarget — local_file", () => {
  it("rejects missing path", () => {
    const [, err] = buildTarget(draft({ type: "local_file" }));
    expect(err).toBe("scheduler.delivery.err_path_required");
  });

  it("rejects absolute Unix paths", () => {
    const [, err] = buildTarget(draft({ type: "local_file", path: "/etc/passwd" }));
    expect(err).toBe("scheduler.delivery.err_path_absolute");
  });

  it("rejects absolute Windows paths", () => {
    const [, err] = buildTarget(draft({ type: "local_file", path: "C:\\Windows\\out.log" }));
    expect(err).toBe("scheduler.delivery.err_path_absolute");
  });

  it("rejects path traversal `..`", () => {
    const [, err] = buildTarget(draft({ type: "local_file", path: "../../etc/passwd" }));
    expect(err).toBe("scheduler.delivery.err_path_traversal");
  });

  it("rejects `..` mid-segment", () => {
    const [, err] = buildTarget(draft({ type: "local_file", path: "logs/../../etc" }));
    expect(err).toBe("scheduler.delivery.err_path_traversal");
  });

  it("accepts workspace-relative paths", () => {
    const [t, err] = buildTarget(draft({ type: "local_file", path: "logs/out.log", append: false }));
    expect(err).toBeNull();
    expect(t).toEqual({ type: "local_file", path: "logs/out.log", append: false });
  });
});

describe("buildTarget — email", () => {
  it("rejects missing recipient", () => {
    const [, err] = buildTarget(draft({ type: "email" }));
    expect(err).toBe("scheduler.delivery.err_email_required");
  });

  it("strips empty subject_template", () => {
    const [t] = buildTarget(draft({ type: "email", to: "a@b.com", subject_template: " " }));
    expect(t).toEqual({ type: "email", to: "a@b.com" });
  });

  it("includes subject_template when provided", () => {
    const [t] = buildTarget(
      draft({ type: "email", to: "a@b.com", subject_template: "Cron: {job}" })
    );
    expect(t).toEqual({ type: "email", to: "a@b.com", subject_template: "Cron: {job}" });
  });
});
