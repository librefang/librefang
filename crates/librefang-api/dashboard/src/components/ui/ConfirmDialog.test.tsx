import React, { useState } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ConfirmDialog } from "./ConfirmDialog";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    }),
  };
});

function DialogHarness({ onConfirm }: { onConfirm: () => void | Promise<void> }) {
  const [isOpen, setIsOpen] = useState(true);
  return (
    <ConfirmDialog
      isOpen={isOpen}
      title="Apply change?"
      message="This action calls the API."
      onConfirm={onConfirm}
      onClose={() => setIsOpen(false)}
    />
  );
}

describe("ConfirmDialog async confirmation", () => {
  it("stays open while pending and allows retry after rejection", async () => {
    let reject!: (reason?: unknown) => void;
    const pending = new Promise<void>((_resolve, rejectPromise) => {
      reject = rejectPromise;
    });
    const onConfirm = vi.fn(() => pending);
    render(<DialogHarness onConfirm={onConfirm} />);

    const confirm = screen.getByRole("button", { name: "Confirm" });
    fireEvent.click(confirm);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    const pendingConfirm = screen.getByRole("button", { name: "Confirm" });
    expect(pendingConfirm).toBeDisabled();
    fireEvent.click(pendingConfirm);
    expect(onConfirm).toHaveBeenCalledTimes(1);

    reject(new Error("API unavailable"));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Confirm" })).toBeEnabled());
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onConfirm).toHaveBeenCalledTimes(2);
  });

  it("closes only after async confirmation succeeds", async () => {
    let resolve!: () => void;
    const pending = new Promise<void>((resolvePromise) => {
      resolve = resolvePromise;
    });
    render(<DialogHarness onConfirm={() => pending} />);

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    resolve();

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("uses the same awaited confirmation path for Enter", async () => {
    let resolve!: () => void;
    const pending = new Promise<void>((resolvePromise) => {
      resolve = resolvePromise;
    });
    const onConfirm = vi.fn(() => pending);
    render(<DialogHarness onConfirm={onConfirm} />);

    fireEvent.keyDown(window, { key: "Enter" });

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeDisabled();

    resolve();

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("does not let a stale success close a reopened confirmation", async () => {
    let resolveFirst!: () => void;
    let resolveSecond!: () => void;
    const first = new Promise<void>((resolve) => { resolveFirst = resolve; });
    const second = new Promise<void>((resolve) => { resolveSecond = resolve; });
    const onClose = vi.fn();
    const common = { title: "Apply?", message: "Confirm action", onClose };
    const { rerender } = render(
      <ConfirmDialog {...common} isOpen onConfirm={() => first} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    rerender(<ConfirmDialog {...common} isOpen={false} onConfirm={() => first} />);
    rerender(<ConfirmDialog {...common} isOpen onConfirm={() => second} />);
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await act(async () => {
      resolveFirst();
      await first;
    });

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeDisabled();

    await act(async () => {
      resolveSecond();
      await second;
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not let a stale rejection unlock a newer confirmation", async () => {
    let rejectFirst!: (reason?: unknown) => void;
    let resolveSecond!: () => void;
    const first = new Promise<void>((_resolve, reject) => { rejectFirst = reject; });
    const second = new Promise<void>((resolve) => { resolveSecond = resolve; });
    const secondConfirm = vi.fn(() => second);
    const onClose = vi.fn();
    const common = { title: "Apply?", message: "Confirm action", onClose };
    const { rerender } = render(
      <ConfirmDialog {...common} isOpen onConfirm={() => first} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    rerender(<ConfirmDialog {...common} isOpen={false} onConfirm={() => first} />);
    rerender(<ConfirmDialog {...common} isOpen onConfirm={secondConfirm} />);
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await act(async () => {
      rejectFirst(new Error("stale failure"));
      try {
        await first;
      } catch {
        // The component consumes the same expected rejection.
      }
    });

    const confirm = screen.getByRole("button", { name: "Confirm" });
    expect(confirm).toBeDisabled();
    fireEvent.click(confirm);
    expect(secondConfirm).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveSecond();
      await second;
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
