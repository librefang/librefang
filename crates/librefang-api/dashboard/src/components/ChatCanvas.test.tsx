import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ChatCanvas } from "./ChatCanvas";
import { CANVAS_CSP } from "../lib/chat";

describe("ChatCanvas", () => {
  it("renders the model HTML in a fully sandboxed iframe behind the canvas CSP", () => {
    render(<ChatCanvas canvas={{ id: "c1", title: "Quarterly table", html: "<table><tr><td>1</td></tr></table>" }} />);

    const frame = screen.getByTitle("Quarterly table");
    expect(frame.tagName).toBe("IFRAME");
    // An empty sandbox token list disables scripts, same-origin access, forms, and popups.
    expect(frame.getAttribute("sandbox")).toBe("");
    const srcdoc = frame.getAttribute("srcdoc") ?? "";
    expect(srcdoc).toContain(CANVAS_CSP);
    expect(srcdoc).toContain("<table><tr><td>1</td></tr></table>");
    expect(screen.getByText("Quarterly table")).toBeTruthy();
  });
});
