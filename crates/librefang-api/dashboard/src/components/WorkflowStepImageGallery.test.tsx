import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { WorkflowStepImageGallery } from "./WorkflowStepImageGallery";
import { extractImageRefs } from "../lib/workflowOutputImages";

describe("WorkflowStepImageGallery", () => {
  const fetchMock = vi.fn();
  const createObjectURL = vi.fn(() => "blob:authenticated-image");
  const revokeObjectURL = vi.fn();

  beforeEach(() => {
    sessionStorage.clear();
    localStorage.clear();
    fetchMock.mockReset();
    fetchMock.mockResolvedValue({
      ok: true,
      blob: async () => new Blob(["image"], { type: "image/png" }),
    } as Response);
    vi.stubGlobal("fetch", fetchMock);
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("renders nothing when refs is empty", () => {
    const { container } = render(<WorkflowStepImageGallery refs={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("fetches upload images with dashboard auth and renders an object URL", async () => {
    sessionStorage.setItem("librefang-api-key", "dashboard-secret");
    const refs = extractImageRefs(
      JSON.stringify({ image_urls: ["/api/uploads/abc-1"] }),
    );
    const { unmount } = render(<WorkflowStepImageGallery refs={refs} />);
    const img = screen.getByRole("img");

    await waitFor(() => {
      expect(img).toHaveAttribute("src", "blob:authenticated-image");
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [path, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe("/api/uploads/abc-1");
    expect(new Headers(init.headers).get("Authorization")).toBe("Bearer dashboard-secret");
    expect(img.closest("a")).toHaveAttribute("href", "blob:authenticated-image");

    unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:authenticated-image");
  });

  it("renders multiple authenticated images as a gallery", async () => {
    const refs = extractImageRefs(
      JSON.stringify({
        image_urls: ["/api/uploads/a", "/api/uploads/b"],
      }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);
    const imgs = screen.getAllByRole("img");
    expect(imgs).toHaveLength(2);
    await waitFor(() => {
      expect(imgs.map((i) => i.getAttribute("src"))).toEqual([
        "blob:authenticated-image",
        "blob:authenticated-image",
      ]);
    });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("leaves remote image URLs unchanged without sending dashboard credentials", () => {
    const refs = extractImageRefs(
      JSON.stringify({ image_urls: ["https://images.example.test/sunset.png"] }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);

    expect(screen.getByRole("img")).toHaveAttribute("src", "https://images.example.test/sunset.png");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uses revised_prompt as alt text when present", () => {
    const refs = extractImageRefs(
      JSON.stringify({
        revised_prompt: "a watercolor sunset",
        image_urls: ["/api/uploads/sunset"],
      }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);
    expect(screen.getByAltText("a watercolor sunset")).toBeInTheDocument();
  });

  it("does not link data URI images to a blocked top-level navigation", () => {
    const refs = extractImageRefs(
      JSON.stringify({ url: "data:image/png;base64,aGVsbG8=" }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);

    expect(screen.getByRole("img").closest("a")).toBeNull();
  });

  it("renders duplicate image refs without duplicate React keys", () => {
    const onError = vi.spyOn(console, "error").mockImplementation(() => {});
    // A remote URL keeps this render synchronous: the duplicate-key warning is a render-time console.error, and an authenticated path would resolve its object URL after the assertions.
    const ref = {
      kind: "url" as const,
      src: "https://images.example.test/duplicate.png",
    };

    render(<WorkflowStepImageGallery refs={[ref, ref]} />);

    expect(screen.getAllByRole("img")).toHaveLength(2);
    expect(onError).not.toHaveBeenCalled();
    onError.mockRestore();
  });

  it("does NOT render anything for plain text (falls back to caller)", () => {
    const refs = extractImageRefs("Just a regular workflow result, no image.");
    const { container } = render(<WorkflowStepImageGallery refs={refs} />);
    expect(container.firstChild).toBeNull();
  });
});
