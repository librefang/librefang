import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { WorkflowStepImageGallery } from "./WorkflowStepImageGallery";
import { extractImageRefs } from "../lib/workflowOutputImages";

describe("WorkflowStepImageGallery", () => {
  it("renders nothing when refs is empty", () => {
    const { container } = render(<WorkflowStepImageGallery refs={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders a single <img> for one image ref", () => {
    const refs = extractImageRefs(
      JSON.stringify({ image_urls: ["/api/uploads/abc-1"] }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);
    const imgs = screen.getAllByRole("img");
    expect(imgs).toHaveLength(1);
    expect(imgs[0]).toHaveAttribute("src", "/api/uploads/abc-1");
  });

  it("renders multiple <img>s as a gallery", () => {
    const refs = extractImageRefs(
      JSON.stringify({
        image_urls: ["/api/uploads/a", "/api/uploads/b"],
      }),
    );
    render(<WorkflowStepImageGallery refs={refs} />);
    const imgs = screen.getAllByRole("img");
    expect(imgs).toHaveLength(2);
    expect(imgs.map((i) => i.getAttribute("src"))).toEqual([
      "/api/uploads/a",
      "/api/uploads/b",
    ]);
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

  it("does NOT render anything for plain text (falls back to caller)", () => {
    const refs = extractImageRefs("Just a regular workflow result, no image.");
    const { container } = render(<WorkflowStepImageGallery refs={refs} />);
    expect(container.firstChild).toBeNull();
  });
});
