import { describe, it, expect } from "vitest";
import { classifyImageRef, extractImageRefs } from "./workflowOutputImages";

describe("classifyImageRef", () => {
  it("accepts http(s) URLs with known image extensions", () => {
    expect(classifyImageRef("https://example.com/foo.png")?.kind).toBe("url");
    expect(classifyImageRef("https://example.com/foo.jpg")?.kind).toBe("url");
    expect(classifyImageRef("https://example.com/foo.jpeg")?.kind).toBe("url");
    expect(classifyImageRef("https://example.com/foo.webp")?.kind).toBe("url");
    expect(classifyImageRef("https://example.com/foo.gif")?.kind).toBe("url");
    expect(classifyImageRef("http://example.com/x.svg?v=1")?.kind).toBe("url");
  });

  it("accepts data:image URIs", () => {
    const ref = classifyImageRef("data:image/png;base64,iVBORw0KGgo=");
    expect(ref?.kind).toBe("data-uri");
    expect(ref?.src).toBe("data:image/png;base64,iVBORw0KGgo=");
  });

  it("accepts /api/media/artifacts/<id> paths", () => {
    const ref = classifyImageRef("/api/media/artifacts/abc123");
    expect(ref?.kind).toBe("artifact");
    expect(ref?.src).toBe("/api/media/artifacts/abc123");
  });

  it("accepts /api/uploads/<id> paths (image_generate tool output)", () => {
    const ref = classifyImageRef("/api/uploads/9d8e7c0a-1111-2222-3333-444455556666");
    expect(ref?.kind).toBe("url");
  });

  it("rejects javascript: scheme", () => {
    expect(classifyImageRef("javascript:alert(1)")).toBeNull();
    expect(classifyImageRef("JavaScript:alert(1)")).toBeNull();
  });

  it("rejects file: scheme", () => {
    expect(classifyImageRef("file:///etc/passwd")).toBeNull();
  });

  it("rejects about: scheme", () => {
    expect(classifyImageRef("about:blank")).toBeNull();
  });

  it("rejects vbscript: scheme", () => {
    expect(classifyImageRef("vbscript:msgbox(1)")).toBeNull();
  });

  it("rejects protocol-relative URLs", () => {
    expect(classifyImageRef("//evil.com/pwn.png")).toBeNull();
  });

  it("rejects non-image data: URIs", () => {
    expect(classifyImageRef("data:text/html,<script>alert(1)</script>")).toBeNull();
    expect(classifyImageRef("data:application/json;base64,e30=")).toBeNull();
  });

  it("rejects http(s) URLs that don't look like images", () => {
    // Non-image extensions and bare hosts: caller should fall back to text.
    expect(classifyImageRef("https://example.com/article.html")).toBeNull();
    expect(classifyImageRef("https://example.com/")).toBeNull();
  });

  it("rejects empty / whitespace", () => {
    expect(classifyImageRef("")).toBeNull();
    expect(classifyImageRef("   ")).toBeNull();
  });
});

describe("extractImageRefs", () => {
  it("returns [] for plain non-image text", () => {
    expect(extractImageRefs("Hello, world!")).toEqual([]);
    expect(extractImageRefs("")).toEqual([]);
    expect(extractImageRefs(null)).toEqual([]);
    expect(extractImageRefs(undefined)).toEqual([]);
  });

  it("returns [] for non-image JSON", () => {
    expect(extractImageRefs(JSON.stringify({ ok: true, count: 3 }))).toEqual([]);
  });

  it("extracts a single URL from { url: ... }", () => {
    const out = extractImageRefs(JSON.stringify({ url: "https://cdn.example/x.png" }));
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: "url", src: "https://cdn.example/x.png" });
  });

  it("extracts the image_generate tool's image_urls array", () => {
    // Matches the shape produced by tool_image_generate in
    // crates/librefang-runtime/src/tool_runner.rs:6592 — keys are
    // images_generated, saved_to, revised_prompt, image_urls.
    const toolJson = JSON.stringify({
      model: "dall-e-3",
      provider: "openai",
      images_generated: 2,
      saved_to: ["/tmp/a.png", "/tmp/b.png"],
      revised_prompt: "a cat in sunglasses",
      image_urls: ["/api/uploads/aaa-111", "/api/uploads/bbb-222"],
    });
    const out = extractImageRefs(toolJson);
    expect(out).toHaveLength(2);
    expect(out.map((r) => r.src)).toEqual([
      "/api/uploads/aaa-111",
      "/api/uploads/bbb-222",
    ]);
    expect(out[0].alt).toBe("a cat in sunglasses");
  });

  it("extracts from the OpenAI generate_image legacy shape { images: [{ url, data_base64 }] }", () => {
    // Matches /api/media/image route handler in
    // crates/librefang-api/src/routes/media.rs:175.
    const json = JSON.stringify({
      images: [
        { url: "/api/uploads/img-1" },
        { url: "https://cdn.openai.com/foo.png", revised_prompt: "x" },
      ],
    });
    const out = extractImageRefs(json);
    expect(out.map((r) => r.src)).toEqual([
      "/api/uploads/img-1",
      "https://cdn.openai.com/foo.png",
    ]);
  });

  it("extracts artifact references ({ artifact } and { artifact_id })", () => {
    const out1 = extractImageRefs(JSON.stringify({ artifact: "/api/media/artifacts/x1" }));
    expect(out1).toEqual([{ kind: "artifact", src: "/api/media/artifacts/x1", alt: undefined }]);

    const out2 = extractImageRefs(JSON.stringify({ artifact_id: "abc-123" }));
    expect(out2).toHaveLength(1);
    expect(out2[0].kind).toBe("artifact");
    expect(out2[0].src).toBe("/api/media/artifacts/abc-123");
  });

  it("extracts data URIs from a nested data_base64 field", () => {
    const out = extractImageRefs(
      JSON.stringify({ images: [{ data_base64: "AAAA", mime: "image/png" }] }),
    );
    expect(out).toHaveLength(1);
    expect(out[0].kind).toBe("data-uri");
    expect(out[0].src).toBe("data:image/png;base64,AAAA");
  });

  it("extracts bare URLs embedded in agent prose", () => {
    const text =
      "I generated the cover. Here you go: https://cdn.example/cover.png — enjoy!";
    const out = extractImageRefs(text);
    expect(out).toHaveLength(1);
    expect(out[0].src).toBe("https://cdn.example/cover.png");
  });

  it("extracts /api/uploads paths embedded in prose", () => {
    const text = "Saved to /api/uploads/9d8e-img — let me know.";
    const out = extractImageRefs(text);
    expect(out.map((r) => r.src)).toContain("/api/uploads/9d8e-img");
  });

  it("extracts data:image URIs from prose", () => {
    const text =
      "preview: data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNgYAAAAAMAAWgmWQ0AAAAASUVORK5CYII=";
    const out = extractImageRefs(text);
    expect(out.some((r) => r.kind === "data-uri")).toBe(true);
  });

  it("handles arrays at the root", () => {
    const out = extractImageRefs(
      JSON.stringify([
        { url: "https://example.com/a.png" },
        { url: "https://example.com/b.jpg" },
      ]),
    );
    expect(out).toHaveLength(2);
  });

  it("handles deeply nested arrays", () => {
    const out = extractImageRefs(
      JSON.stringify({
        results: { gallery: { items: [{ url: "https://example.com/x.webp" }] } },
      }),
    );
    expect(out).toHaveLength(1);
    expect(out[0].src).toBe("https://example.com/x.webp");
  });

  it("dedupes repeated URLs", () => {
    const out = extractImageRefs(
      JSON.stringify({
        image_urls: ["/api/uploads/x", "/api/uploads/x", "/api/uploads/x"],
      }),
    );
    expect(out).toHaveLength(1);
  });

  it("rejects dangerous schemes embedded as a URL field", () => {
    // The classifier short-circuits even when the JSON is otherwise valid.
    const out = extractImageRefs(
      JSON.stringify({
        url: "javascript:alert(1)",
        also: "data:text/html,<script>",
      }),
    );
    expect(out).toEqual([]);
  });

  it("rejects protocol-relative URLs in JSON fields", () => {
    const out = extractImageRefs(JSON.stringify({ url: "//evil.com/pwn.png" }));
    expect(out).toEqual([]);
  });

  it("rejects file: scheme in JSON fields", () => {
    const out = extractImageRefs(JSON.stringify({ url: "file:///etc/passwd" }));
    expect(out).toEqual([]);
  });

  it("recovers an image URL even when prose contains a malformed JSON block", () => {
    // The agent sometimes writes broken JSON. Bare-URL scan still works.
    const text =
      "Here's the result: { not really json } but the image is at https://example.com/result.png";
    const out = extractImageRefs(text);
    expect(out.map((r) => r.src)).toContain("https://example.com/result.png");
  });

  it("strips trailing punctuation from bare URLs in prose", () => {
    const out = extractImageRefs(
      "see https://example.com/foo.png, also https://example.com/bar.jpg.",
    );
    expect(out.map((r) => r.src)).toEqual([
      "https://example.com/foo.png",
      "https://example.com/bar.jpg",
    ]);
  });

  it("accepts a pre-parsed object input", () => {
    const out = extractImageRefs({ url: "https://example.com/parsed.png" });
    expect(out).toHaveLength(1);
    expect(out[0].src).toBe("https://example.com/parsed.png");
  });
});
