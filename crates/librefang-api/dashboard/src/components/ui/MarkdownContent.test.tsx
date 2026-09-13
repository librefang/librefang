import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MarkdownContent, urlTransform } from "./MarkdownContent";

// Mermaid needs real SVG layout, which jsdom does not provide, so the library
// is stubbed at its module boundary. What these tests own is the wiring: that a
// ```mermaid fence reaches Mermaid at all, that the rendered SVG is mounted,
// and that a source Mermaid rejects degrades to the code block rather than an
// error — not Mermaid's own rendering, which is its project's business.
const renderDiagram = vi.fn();
const initialize = vi.fn();
vi.mock("mermaid", () => ({
  default: {
    initialize: (...args: unknown[]) => initialize(...args),
    render: (...args: unknown[]) => renderDiagram(...args),
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
  }),
}));

vi.mock("../../lib/store", () => ({
  useUIStore: (selector: (s: { theme: string }) => unknown) => selector({ theme: "dark" }),
}));

describe("MarkdownContent urlTransform", () => {
  it.each([
    "obsidian://open?vault=Notes&file=Projects%2FLibreFang.md",
    "obsidian-advanced-uri://open?vault=Notes&filepath=Daily%20Notes%2Ftoday.md#Tasks",
  ])("allows a valid Obsidian deep link: %s", (url) => {
    expect(urlTransform(url)).toBe(url);
  });

  it.each([
    'obsidian://open?vault=Notes" onclick="alert(1)',
    "obsidian://open?vault=Notes and file=secret",
    "obsidian://open?vault=Notes\\file=secret",
    "obsidian://open?vault=Notes\u0000file=secret",
  ])("rejects invalid characters in an Obsidian deep link", (url) => {
    expect(urlTransform(url)).toBe("");
  });

  it("retains the default URL policy for other schemes", () => {
    expect(urlTransform("https://example.com/docs")).toBe("https://example.com/docs");
    expect(urlTransform("javascript:alert(1)")).toBe("");
  });
});

describe("MarkdownContent mermaid blocks", () => {
  const DIAGRAM = "graph TD;\n  A-->B;";

  beforeEach(() => {
    vi.clearAllMocks();
    renderDiagram.mockResolvedValue({ svg: '<svg data-testid="svg-body"></svg>' });
  });

  it("renders a mermaid fence as a diagram", async () => {
    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);

    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());
    expect(screen.getByTestId("svg-body")).toBeInTheDocument();
    // The fence body reaches Mermaid without its trailing newline.
    expect(renderDiagram).toHaveBeenCalledWith(expect.any(String), DIAGRAM);
  });

  it("initialises mermaid with securityLevel strict and a pinned layout", async () => {
    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);

    await waitFor(() => expect(initialize).toHaveBeenCalled());
    expect(initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        securityLevel: "strict",
        // `layout` is not protected by mermaid's own `secure` defaults, so an
        // `%%{init: {"layout":"elk"}}%%` line in untrusted diagram source could
        // otherwise pull a 1.4 MB layout engine.
        layout: "dagre",
        secure: expect.arrayContaining(["securityLevel", "layout"]),
      }),
    );
  });

  it("falls back to the source when the diagram does not render", async () => {
    renderDiagram.mockRejectedValue(new Error("Parse error on line 2"));

    render(<MarkdownContent diagrams>{"```mermaid\nnot a diagram\n```"}</MarkdownContent>);

    await waitFor(() => expect(screen.getByTestId("mermaid-source")).toBeInTheDocument());
    expect(screen.getByText("not a diagram")).toBeInTheDocument();
    expect(screen.queryByTestId("mermaid-diagram")).not.toBeInTheDocument();
  });

  // An empty SVG was the one input that degraded into a blank box announced as
  // a diagram, with the source gone.
  it.each([{ svg: "" }, {}])("falls back when render resolves without an SVG: %o", async (result) => {
    renderDiagram.mockResolvedValue(result);

    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);

    await waitFor(() => expect(screen.getByTestId("mermaid-source")).toBeInTheDocument());
    expect(screen.queryByTestId("mermaid-diagram")).not.toBeInTheDocument();
  });

  it("gives every diagram in a message its own render id", async () => {
    render(
      <MarkdownContent diagrams>
        {"```mermaid\n" + DIAGRAM + "\n```\n\n```mermaid\ngraph LR;\n  C-->D;\n```"}
      </MarkdownContent>,
    );

    await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(2));
    const ids = renderDiagram.mock.calls.map((c) => c[0]);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("does not re-render the diagram when the parent re-renders with the same source", async () => {
    const markup = "```mermaid\n" + DIAGRAM + "\n```";
    const { rerender } = render(<MarkdownContent diagrams>{markup}</MarkdownContent>);
    await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(1));

    rerender(<MarkdownContent diagrams>{markup}</MarkdownContent>);
    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());
    expect(renderDiagram).toHaveBeenCalledTimes(1);
  });

  // This renderer is shared with a memory record's 4rem preview box and the
  // chat's thinking panel, which grows on every streamed frame.
  it("leaves the fence as a code block unless diagrams are asked for", () => {
    const { container } = render(
      <MarkdownContent>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>,
    );

    expect(container.querySelector("pre > code")?.textContent).toContain("graph TD;");
    expect(screen.queryByTestId("mermaid-diagram")).not.toBeInTheDocument();
    expect(renderDiagram).not.toHaveBeenCalled();
  });

  it("applies the app's link policy to anchors mermaid emits", async () => {
    renderDiagram.mockResolvedValue({
      svg: '<svg><a xlink:href="https://evil.example/phish" target="_blank">n</a></svg>',
    });

    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);

    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());
    expect(screen.getByTestId("mermaid-diagram").querySelector("a"))
      .toHaveAttribute("rel", "noopener noreferrer");
  });

  it("leaves every other fenced language as a code block", async () => {
    render(<MarkdownContent diagrams>{"```rust\nfn main() {}\n```"}</MarkdownContent>);

    expect(screen.getByText("fn main() {}")).toBeInTheDocument();
    expect(screen.queryByTestId("mermaid-diagram")).not.toBeInTheDocument();
    expect(screen.queryByTestId("mermaid-source")).not.toBeInTheDocument();
    expect(renderDiagram).not.toHaveBeenCalled();
  });

  // `language-mermaidish` must not be mistaken for the real fence, and a
  // capitalised one must still work — agents write both.
  it("matches the language token exactly, and case-insensitively", async () => {
    const { unmount } = render(<MarkdownContent diagrams>{"```mermaidish\nx\n```"}</MarkdownContent>);
    expect(screen.queryByTestId("mermaid-diagram")).not.toBeInTheDocument();
    expect(renderDiagram).not.toHaveBeenCalled();
    unmount();

    render(<MarkdownContent diagrams>{"```Mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);
    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());
  });
});

describe("MermaidDiagram — fitting the chat column", () => {
  const DIAGRAM = "graph TD;\n  A-->B;";

  beforeEach(() => {
    vi.clearAllMocks();
    // Mermaid writes its own width onto the <svg>, which is the thing that has
    // to lose to the container's cap.
    renderDiagram.mockResolvedValue({
      svg: '<svg data-testid="svg-body" style="max-width: 1840px;" width="100%"></svg>',
    });
  });

  it("caps the diagram at the container width with a rule that beats mermaid's inline style", async () => {
    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);
    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());

    const box = screen.getByTestId("mermaid-diagram");
    // An inline `style="max-width: 1840px"` outranks a plain class, so the cap
    // has to carry the important modifier or a wide diagram walks out of the
    // chat column with nothing to scroll.
    expect(box.className).toContain("[&>svg]:!max-w-full");
    // And the container is what scrolls when the diagram still cannot shrink.
    expect(box.className).toContain("overflow-x-auto");
    expect(box.className).toContain("min-w-0");
    // Bounded in height too, so a tall diagram is a thumbnail in the
    // transcript rather than a page of its own.
    expect(box.className).toContain("[&>svg]:!max-h-64");
    // Released from mermaid's `width="100%"`, or the height cap would squash
    // the diagram instead of scaling it.
    expect(box.className).toContain("[&>svg]:!w-auto");
  });

  it("follows a diagram link instead of opening the modal", async () => {
    const user = userEvent.setup();
    // What mermaid emits for a `click X "url" _blank` directive.
    renderDiagram.mockResolvedValue({
      svg: '<svg data-testid="svg-body"><a href="https://example.com/x" target="_blank">node</a></svg>',
    });
    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);
    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());

    const link = screen.getByTestId("mermaid-diagram").querySelector("a")!;
    await user.click(link);

    // Clicking a link should not also enlarge the diagram behind it.
    expect(screen.queryByTestId("mermaid-diagram-zoomed")).not.toBeInTheDocument();

    // The enlarge control still works from the same diagram.
    await user.click(screen.getByRole("button", { name: "Enlarge diagram" }));
    expect(await screen.findByTestId("mermaid-diagram-zoomed")).toBeInTheDocument();
  });

  it("opens the diagram full size on demand, and not before", async () => {
    const user = userEvent.setup();
    render(<MarkdownContent diagrams>{"```mermaid\n" + DIAGRAM + "\n```"}</MarkdownContent>);
    await waitFor(() => expect(screen.getByTestId("mermaid-diagram")).toBeInTheDocument());

    expect(screen.queryByTestId("mermaid-diagram-zoomed")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Enlarge diagram" }));

    const zoomed = await screen.findByTestId("mermaid-diagram-zoomed");
    // Uncapped, unlike the inline copy: this view exists to show the diagram
    // at its natural size, and its own container is what scrolls.
    expect(zoomed.className).not.toContain("max-w-full");
    expect(zoomed.className).toContain("overflow-auto");
    // Same markup, ids untouched — mermaid's <style> block selects on the root
    // id, so renaming it would strip the enlarged copy of its styling.
    expect(zoomed.querySelector('[data-testid="svg-body"]')).not.toBeNull();
  });
});
