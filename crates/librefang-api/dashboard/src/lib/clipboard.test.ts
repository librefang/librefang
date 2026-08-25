import { afterEach, describe, expect, it, vi } from "vitest";

import { copyToClipboard } from "./clipboard";

describe("copyToClipboard", () => {
  afterEach(() => {
    document.body.replaceChildren();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("uses the clipboard API when it succeeds", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });

    await expect(copyToClipboard("hello")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("falls back without making the textarea readonly", async () => {
    vi.stubGlobal("navigator", {});
    let wasReadonly = true;
    const execCommand = vi.fn(() => {
      wasReadonly = document.querySelector("textarea")?.readOnly ?? true;
      return true;
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    await expect(copyToClipboard("hello")).resolves.toBe(true);

    expect(execCommand).toHaveBeenCalledOnce();
    expect(wasReadonly).toBe(false);
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("removes the fallback textarea when copying throws", async () => {
    vi.stubGlobal("navigator", {});
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn(() => {
        throw new Error("copy failed");
      }),
    });

    await expect(copyToClipboard("hello")).resolves.toBe(false);
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("removes the fallback textarea when selection throws", async () => {
    vi.stubGlobal("navigator", {});
    vi.spyOn(HTMLTextAreaElement.prototype, "select").mockImplementation(() => {
      throw new Error("selection failed");
    });

    await expect(copyToClipboard("hello")).resolves.toBe(false);
    expect(document.querySelector("textarea")).toBeNull();
  });
});
