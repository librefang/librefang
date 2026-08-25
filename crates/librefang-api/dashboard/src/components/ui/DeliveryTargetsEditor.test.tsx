import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  buildTarget,
  DeliveryTargetsEditor,
  type DraftState,
} from "./DeliveryTargetsEditor";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  }),
}));

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

  it.each([
    "http://0.1.2.3/h",
    "http://10.0.0.1/h",
    "http://100.64.0.1/h",
    "http://172.16.0.1/h",
    "http://192.168.0.1/h",
  ])("rejects blocked IPv4 range %s", (url) => {
    const [, err] = buildTarget(draft({ type: "webhook", url }));
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it.each([
    "http://[::]/h",
    "http://[fd00::1]/h",
    "http://[fe90::1]/h",
    "http://[ff02::1]/h",
    "http://[::ffff:127.0.0.1]/h",
    "http://[::ffff:10.0.0.1]/h",
  ])("rejects blocked IPv6 range %s", (url) => {
    const [, err] = buildTarget(draft({ type: "webhook", url }));
    expect(err).toBe("scheduler.delivery.err_url_blocked_host");
  });

  it.each([
    "http://instance-data/h",
    "http://api.localhost/h",
    "http://service.internal/h",
  ])("rejects blocked internal hostname %s", (url) => {
    const [, err] = buildTarget(draft({ type: "webhook", url }));
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

  it("accepts a case-mixed HTTP scheme after URL normalization", () => {
    const [t, err] = buildTarget(
      draft({ type: "webhook", url: "HTTPS://example.com/hook" }),
    );
    expect(err).toBeNull();
    expect(t).toEqual({ type: "webhook", url: "HTTPS://example.com/hook" });
  });

  it.each([
    "http://100.63.255.255/h",
    "http://100.128.0.1/h",
    "http://169.253.0.1/h",
    "http://172.15.255.255/h",
    "http://172.32.0.1/h",
  ])("accepts public IPv4 boundary %s", (url) => {
    const [target, err] = buildTarget(draft({ type: "webhook", url }));
    expect(err).toBeNull();
    expect(target).toEqual({ type: "webhook", url });
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

  it("prompts for a workspace-relative path", async () => {
    const user = userEvent.setup();
    render(<DeliveryTargetsEditor value={[]} onChange={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Add target" }));
    await user.click(screen.getByRole("button", { name: "Local file" }));

    expect(screen.getByPlaceholderText("logs/cron-output.log")).toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText("/var/log/cron-output.log"),
    ).not.toBeInTheDocument();
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
