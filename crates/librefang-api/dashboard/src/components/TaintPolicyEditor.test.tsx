/**
 * Regression tests for #5799: toggling taint_scanning on one MCP server
 * must not affect the displayed state for other servers.
 *
 * Root cause: TaintPolicyEditor used `useState(server.taint_scanning ?? true)`
 * without any mechanism to reset when the server prop changed. The fix adds
 * a `useEffect` that syncs the local scanning/tools state whenever
 * `server.name`, `server.taint_scanning`, or `server.taint_policy` changes,
 * and a `key={server.id ?? server.name}` in the parent that forces a full
 * remount when a different server is selected.
 */

import { describe, it, expect, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import React from "react";
import { TaintPolicyEditor } from "./TaintPolicyEditor";
import type { McpServerConfigured } from "../api";

// Minimal transport fixture that satisfies the type.
const STDIO_TRANSPORT = { type: "stdio" as const, command: "npx", args: [] };

function makeServer(name: string, taint_scanning: boolean): McpServerConfigured {
  return {
    id: name,
    name,
    transport: STDIO_TRANSPORT,
    timeout_secs: 30,
    taint_scanning,
    taint_policy: undefined,
  };
}

// ── mock heavy dependencies ──────────────────────────────────────────────

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, defaultOrOpts?: unknown) => {
      if (typeof defaultOrOpts === "string") return defaultOrOpts;
      if (
        defaultOrOpts &&
        typeof defaultOrOpts === "object" &&
        "defaultValue" in (defaultOrOpts as Record<string, unknown>)
      ) {
        return String((defaultOrOpts as { defaultValue: string }).defaultValue);
      }
      return key;
    },
  }),
}));

vi.mock("../lib/mutations/mcp", () => ({
  useUpdateMcpTaintPolicy: () => ({ mutate: vi.fn(), isPending: false }),
}));

vi.mock("../lib/queries/mcp", () => ({
  useMcpTaintRules: () => ({ data: [] }),
}));

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: () => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

// DrawerPanel just renders children when isOpen=true.
vi.mock("./ui/DrawerPanel", () => ({
  DrawerPanel: ({
    children,
    isOpen,
  }: {
    children: React.ReactNode;
    isOpen: boolean;
  }) => (isOpen ? <div data-testid="drawer">{children}</div> : null),
}));

vi.mock("./ui/Button", () => ({
  Button: ({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) => (
    <button onClick={onClick} disabled={disabled}>
      {children}
    </button>
  ),
}));

vi.mock("./ui/Badge", () => ({
  Badge: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

vi.mock("./ui/Input", () => ({
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("./ui/Select", () => ({
  Select: (props: React.SelectHTMLAttributes<HTMLSelectElement> & { options: { value: string; label: string }[] }) => (
    <select value={props.value} onChange={props.onChange}>
      {(props.options ?? []).map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  ),
}));

// ── helpers ──────────────────────────────────────────────────────────────

/**
 * Returns true when the rendered editor is showing the ShieldCheck icon,
 * indicating taint_scanning=true. ShieldOff appears when scanning=false.
 */
function isScanningOn(): boolean {
  // ShieldCheck renders as an svg with a data-testid or class we can find.
  // Both icons are lucide — in jsdom they render as <svg> with a title.
  // Simpler: look for the "off" warning banner that only appears when
  // scanning=false (the mcp.taint_scanning_off_warning paragraph).
  return screen.queryByText(/Per-tool exemptions below are ignored/) === null;
}

// ── tests ─────────────────────────────────────────────────────────────────

describe("TaintPolicyEditor — taint_scanning state sync (regression #5799)", () => {
  it("initialises scanning=true when server.taint_scanning is true", () => {
    const server = makeServer("server-a", true);
    render(
      <TaintPolicyEditor server={server} isOpen onClose={vi.fn()} />,
    );
    expect(isScanningOn()).toBe(true);
  });

  it("initialises scanning=false when server.taint_scanning is false", () => {
    const server = makeServer("server-a", false);
    render(
      <TaintPolicyEditor server={server} isOpen onClose={vi.fn()} />,
    );
    expect(isScanningOn()).toBe(false);
  });

  it("resets scanning when server prop changes to a different server with taint_scanning=true", () => {
    // Simulate the parent rendering server-a (scanning OFF), then swapping to
    // server-b (scanning ON). Without the useEffect fix the component would
    // keep showing scanning=false even for server-b.
    const serverA = makeServer("server-a", false);
    const serverB = makeServer("server-b", true);

    const { rerender } = render(
      <TaintPolicyEditor server={serverA} isOpen onClose={vi.fn()} />,
    );
    expect(isScanningOn()).toBe(false);

    act(() => {
      rerender(
        <TaintPolicyEditor server={serverB} isOpen onClose={vi.fn()} />,
      );
    });

    // After prop change, scanning must reflect server-b's value (true).
    expect(isScanningOn()).toBe(true);
  });

  it("resets scanning when server prop changes to a different server with taint_scanning=false", () => {
    const serverA = makeServer("server-a", true);
    const serverB = makeServer("server-b", false);

    const { rerender } = render(
      <TaintPolicyEditor server={serverA} isOpen onClose={vi.fn()} />,
    );
    expect(isScanningOn()).toBe(true);

    act(() => {
      rerender(
        <TaintPolicyEditor server={serverB} isOpen onClose={vi.fn()} />,
      );
    });

    expect(isScanningOn()).toBe(false);
  });
});
