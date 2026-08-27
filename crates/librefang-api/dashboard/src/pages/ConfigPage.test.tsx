import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  ConfigPage,
  JsonEditor,
  configSavePresentation,
  configSectionTabClass,
  configValuesEqual,
  effectiveConfigTab,
} from "./ConfigPage";
import {
  useConfigSchema,
  useConfigStatus,
  useFullConfig,
  useRawConfigToml,
} from "../lib/queries/config";
import {
  useBatchSetConfigValues,
  useSetConfigValue,
  useReloadConfig,
} from "../lib/mutations/config";
import type { ConfigSchemaRoot, ConfigStatus } from "../api";

vi.mock("../lib/queries/config", () => ({
  useConfigSchema: vi.fn(),
  useConfigStatus: vi.fn(),
  useFullConfig: vi.fn(),
  useRawConfigToml: vi.fn(),
}));

vi.mock("../lib/mutations/config", () => ({
  useBatchSetConfigValues: vi.fn(),
  useSetConfigValue: vi.fn(),
  useReloadConfig: vi.fn(),
}));

// The page only uses the router for the category tab strip and the
// unsaved-changes blocker; neither is under test here.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children?: React.ReactNode }) => (
    <a {...rest}>{children}</a>
  ),
  useBlocker: () => ({ status: "IDLE", proceed: vi.fn(), reset: vi.fn() }),
}));

vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    // Return the supplied default so assertions read as the operator-facing
    // English copy rather than as key names.
    useTranslation: () => ({
      t: (key: string, fallback?: unknown) =>
        typeof fallback === "string" ? fallback : key,
    }),
  };
});

const useConfigSchemaMock = vi.mocked(useConfigSchema);
const useConfigStatusMock = vi.mocked(useConfigStatus);
const useFullConfigMock = vi.mocked(useFullConfig);
const useRawConfigTomlMock = vi.mocked(useRawConfigToml);

/**
 * One root-level `general` section with a single editable string field.
 * `log_level` is deliberately a path the write endpoint *does* accept, so a
 * lock observed in these tests can only have come from managed mode and not
 * from the pre-existing `x-non-writable` list.
 */
const SCHEMA: ConfigSchemaRoot = {
  type: "object",
  properties: { log_level: { type: "string", title: "Log Level" } },
  "x-sections": [
    { key: "general", title: "General", root_level: true, fields: ["log_level"] },
  ],
  "x-non-writable": [],
};

/**
 * A settled `useQuery` result carrying `data`.
 *
 * Typed through `unknown` on purpose: the page reads `.data` and the two
 * `isError` / `error` fields off these hooks and nothing else, so reproducing
 * TanStack's full discriminated result union here would be noise that has to
 * be maintained against a library type for no added coverage.
 */
function makeQuery<T>(data: T): unknown {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    isSuccess: true,
    error: null,
    refetch: vi.fn(),
  };
}

function setStatus(status: Partial<ConfigStatus> | undefined) {
  useConfigStatusMock.mockReturnValue(
    makeQuery(status) as ReturnType<typeof useConfigStatus>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();

  useConfigSchemaMock.mockReturnValue(
    makeQuery(SCHEMA) as ReturnType<typeof useConfigSchema>,
  );
  useFullConfigMock.mockReturnValue(
    makeQuery({ log_level: "info" }) as ReturnType<typeof useFullConfig>,
  );
  useRawConfigTomlMock.mockReturnValue(
    makeQuery(undefined) as ReturnType<typeof useRawConfigToml>,
  );

  const mutation = { mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false };
  vi.mocked(useSetConfigValue).mockReturnValue(
    mutation as unknown as ReturnType<typeof useSetConfigValue>,
  );
  vi.mocked(useBatchSetConfigValues).mockReturnValue(
    mutation as unknown as ReturnType<typeof useBatchSetConfigValues>,
  );
  vi.mocked(useReloadConfig).mockReturnValue(
    mutation as unknown as ReturnType<typeof useReloadConfig>,
  );
});

/**
 * The control's own `disabled` attribute is not what locks it — the page wraps
 * the field in an `inert` container so pointer *and* keyboard interaction are
 * blocked for composite editors too. Assert on that wrapper, since asserting
 * `toBeDisabled()` would pass on an input the operator can still type into.
 */
function inertWrapperFor(input: HTMLElement): HTMLElement | null {
  return input.closest("[inert]");
}

describe("ConfigPage managed mode (#6695)", () => {
  it("renders the control as an editable input when the daemon is writable", () => {
    setStatus({ mode: "mutable", source: "/root/.librefang/config.toml", writable: true });

    render(<ConfigPage category="general" />);

    const input = screen.getByLabelText("Log Level");
    expect(inertWrapperFor(input)).toBeNull();
    expect(screen.queryByTestId("managed-config-banner")).toBeNull();
    expect(screen.queryByTestId("locked-reason-log_level")).toBeNull();
  });

  it("renders the control as locked, not as an enabled control that will 423 on save", () => {
    setStatus({
      mode: "managed",
      source: "/etc/librefang/config.toml",
      writable: false,
      checksum: "sha256:9f2b",
    });

    render(<ConfigPage category="general" />);

    // The field is still readable — an operator must be able to see what the
    // deployment set — but it is not offered as editable.
    const input = screen.getByLabelText("Log Level");
    expect(input).toHaveValue("info");
    expect(inertWrapperFor(input)).not.toBeNull();

    // And the lock is explained, rather than left to be discovered on save.
    expect(screen.getByTestId("locked-reason-log_level")).toHaveTextContent(
      "Locked — this daemon's configuration is owned by the deployment.",
    );
  });

  it("names the managed file and its checksum so a rollout can be confirmed", () => {
    setStatus({
      mode: "managed",
      source: "/etc/librefang/config.toml",
      writable: false,
      checksum: "sha256:9f2bdeadbeef",
    });

    render(<ConfigPage category="general" />);

    const banner = screen.getByTestId("managed-config-banner");
    expect(banner).toHaveTextContent("Configuration is managed by the deployment");
    expect(banner).toHaveTextContent("/etc/librefang/config.toml");
    expect(banner).toHaveTextContent("sha256:9f2bdeadbeef");
  });

  it("does not tell a managed operator to edit config.toml and reload", () => {
    // The generic `x-non-writable` copy points at config.toml, which is
    // actively wrong advice for a file the next rollout overwrites.
    setStatus({ mode: "managed", source: "/etc/librefang/config.toml", writable: false });

    render(<ConfigPage category="general" />);

    expect(screen.getByTestId("locked-reason-log_level")).not.toHaveTextContent(
      "change it in config.toml and reload",
    );
  });

  it("discards edits made before the status query reported managed mode", async () => {
    // Fields render editable until `/api/config/status` answers, so an
    // operator who types immediately can accumulate changes that turn out to
    // be unsavable. Stranding them would leave a navigation blocker behind a
    // "Save All" bar that is no longer on screen.
    setStatus({ mode: "mutable", source: "/root/.librefang/config.toml", writable: true });

    const { rerender } = render(<ConfigPage category="general" />);

    await userEvent.clear(screen.getByLabelText("Log Level"));
    await userEvent.type(screen.getByLabelText("Log Level"), "debug");
    expect(screen.getByText("Save All")).toBeInTheDocument();

    setStatus({ mode: "managed", source: "/etc/librefang/config.toml", writable: false });
    rerender(<ConfigPage category="general" />);

    expect(screen.queryByText("Save All")).toBeNull();
    // The field falls back to the server value, not the abandoned edit.
    expect(screen.getByLabelText("Log Level")).toHaveValue("info");
  });

  it("stays editable when the status query has not answered", () => {
    // An older daemon has no `/api/config/status`. Failing open costs at worst
    // one honest 423 on save; failing closed would grey out every control with
    // no way for the operator to tell why.
    setStatus(undefined);

    render(<ConfigPage category="general" />);

    expect(inertWrapperFor(screen.getByLabelText("Log Level"))).toBeNull();
    expect(screen.queryByTestId("managed-config-banner")).toBeNull();
  });
});

describe("ConfigPage form state helpers", () => {
  it("preserves an invalid JSON draft across server value updates", () => {
    const onChange = vi.fn();
    const view = render(<JsonEditor value={{ enabled: true }} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    fireEvent.change(editor, { target: { value: '{"enabled":' } });
    view.rerender(<JsonEditor value={{ enabled: false }} onChange={onChange} />);

    expect(editor).toHaveValue('{"enabled":');
    expect(onChange).not.toHaveBeenCalled();
  });

  it("compares nested config values without object key-order sensitivity", () => {
    expect(
      configValuesEqual(
        { outer: { alpha: 1, beta: [2, 3] } },
        { outer: { beta: [2, 3], alpha: 1 } },
      ),
    ).toBe(true);
    expect(configValuesEqual({ alpha: 1 }, { alpha: 2 })).toBe(false);
  });

  it("selects explicit save status branches", () => {
    const t = (key: string, fallback: string) => `${key}:${fallback}`;

    expect(
      configSavePresentation(
        { status: "saved_reload_failed", reload_error: "bad TOML" },
        t,
      ),
    ).toEqual({
      ok: false,
      msg: "config.saved_reload_failed:Saved but reload failed: bad TOML",
    });
    expect(configSavePresentation({ restart_required: true }, t).ok).toBe(true);
  });

  it("derives effective sections and tab styling without nested branches", () => {
    expect(effectiveConfigTab(true, "general", ["general"])).toBeNull();
    expect(effectiveConfigTab(false, "missing", ["general"])).toBe("general");
    expect(configSectionTabClass(true, false)).toBe("border-brand text-brand");
    expect(configSectionTabClass(false, true)).toContain("cursor-not-allowed");
  });
});
