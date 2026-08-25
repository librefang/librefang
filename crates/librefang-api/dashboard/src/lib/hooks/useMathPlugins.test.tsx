import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

describe("useMathPlugins", () => {
  it("detects supported math delimiters", async () => {
    const { containsMathDelimiters } = await import("./useMathPlugins");

    expect(containsMathDelimiters("plain text")).toBe(false);
    expect(containsMathDelimiters("inline $x + y$ math")).toBe(true);
    expect(containsMathDelimiters("display \\[x + y\\]")).toBe(true);
  });

  it("loads plugins only after math appears", async () => {
    const { useMathPlugins } = await import("./useMathPlugins");
    const { result, rerender } = renderHook(
      ({ content }) => useMathPlugins(content),
      { initialProps: { content: "plain" } },
    );

    expect(result.current.remarkPlugins).toEqual([]);
    rerender({ content: "now $x + y$" });

    await waitFor(() => expect(result.current.remarkPlugins).toHaveLength(1));
    expect(result.current.rehypePlugins).toHaveLength(1);
  });

  it("handles a rejected plugin load and retries later", async () => {
    vi.resetModules();
    vi.doMock("remark-math", () => {
      throw new Error("chunk unavailable");
    });
    const { useMathPlugins } = await import("./useMathPlugins");
    const { result, rerender } = renderHook(
      ({ content }) => useMathPlugins(content),
      { initialProps: { content: "$x + y$" } },
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(result.current).toEqual({ remarkPlugins: [], rehypePlugins: [] });

    vi.doMock("remark-math", () => ({ default: vi.fn() }));
    rerender({ content: "plain" });
    rerender({ content: "$retry$" });
    await waitFor(() => expect(result.current.remarkPlugins).toHaveLength(1));

    vi.doUnmock("remark-math");
    vi.resetModules();
  });
});
