import { describe, it, expect, vi, afterEach } from "vitest";
import { downloadBlob, DOWNLOAD_REVOKE_DELAY_MS } from "./download";

describe("downloadBlob", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("clicks an attached anchor and revokes the object URL only after a delay", () => {
    vi.useFakeTimers();
    const createObjectURL = vi.fn(() => "blob:download-test");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", { ...URL, createObjectURL, revokeObjectURL });
    const clicks: { anchor: HTMLAnchorElement; attached: boolean }[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
      clicks.push({ anchor: this, attached: document.body.contains(this) });
    });

    const blob = new Blob(["x"], { type: "text/plain" });
    downloadBlob(blob, "file.txt");

    expect(createObjectURL).toHaveBeenCalledWith(blob);
    expect(clicks).toHaveLength(1);
    const [{ anchor, attached }] = clicks;
    expect(anchor.download).toBe("file.txt");
    expect(anchor.getAttribute("href")).toBe("blob:download-test");
    expect(attached).toBe(true);
    expect(document.body.contains(anchor)).toBe(false);
    // Revoking in the same task as the click can cancel the download in Safari.
    expect(revokeObjectURL).not.toHaveBeenCalled();

    vi.advanceTimersByTime(DOWNLOAD_REVOKE_DELAY_MS);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:download-test");
  });
});
