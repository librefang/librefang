import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it.each([
    ["Jane Doe", "JD"],
    ["  Jane   Doe  ", "JD"],
    ["Jane\tDoe", "JD"],
    ["Jane\nDoe", "JD"],
    ["Jane\u00a0Doe", "JD"],
    ["Jane", "J"],
    ["   ", "?"],
  ])("derives initials from %j", (fallback, expected) => {
    render(<Avatar fallback={fallback} />);

    const avatar = screen.getByRole("img");
    expect(avatar).toHaveAttribute("aria-label", fallback);
    expect(avatar).toHaveTextContent(expected);
  });

  // Image, then emoji, then initials — the order in which the identity was
  // deliberately set (#8339).
  describe("precedence", () => {
    it("shows the image when there is one, over both the emoji and the initials", () => {
      render(<Avatar fallback="Jane Doe" emoji="🤖" src="blob:test/1" />);

      const image = screen.getByRole("img", { name: "Jane Doe" }).querySelector("img");
      expect(image).toHaveAttribute("src", "blob:test/1");
      expect(screen.getByRole("img", { name: "Jane Doe" })).not.toHaveTextContent("🤖");
      expect(screen.getByRole("img", { name: "Jane Doe" })).not.toHaveTextContent("JD");
    });

    it("shows the emoji when there is no image", () => {
      render(<Avatar fallback="Jane Doe" emoji="🤖" />);

      const avatar = screen.getByRole("img");
      expect(avatar).toHaveTextContent("🤖");
      expect(avatar).not.toHaveTextContent("JD");
    });

    it("falls back to the initials when there is neither", () => {
      render(<Avatar fallback="Jane Doe" />);

      expect(screen.getByRole("img")).toHaveTextContent("JD");
    });

    it("keeps the name as the accessible label, never the emoji", () => {
      render(<Avatar fallback="Jane Doe" emoji="🤖" />);

      // The emoji is decoration on a control that already announces the agent.
      // Passing it through `fallback` — the only text prop before #8339 — would
      // have made a screen reader read the picture instead of the name.
      expect(screen.getByRole("img")).toHaveAttribute("aria-label", "Jane Doe");
    });

    it("renders a multi-codepoint emoji whole", () => {
      // The reason `emoji` is its own prop: `getInitials` takes `n[0]`, the
      // first UTF-16 code unit, so a family sequence would come out as a lone
      // surrogate. Eleven code units in, eleven out.
      const family = "👨‍👩‍👧‍👦";
      render(<Avatar fallback="Jane Doe" emoji={family} />);

      expect(screen.getByRole("img")).toHaveTextContent(family);
    });

    it("falls through to the emoji when the image fails to load", () => {
      render(<Avatar fallback="Jane Doe" emoji="🤖" src="blob:test/gone" />);

      const avatar = screen.getByRole("img", { name: "Jane Doe" });
      fireEvent.error(avatar.querySelector("img")!);

      // A stored avatar whose file is gone is a reachable state — restoring a
      // database without the avatars directory produces it — and the identity
      // still has an emoji to show.
      expect(avatar).toHaveTextContent("🤖");
    });
  });
});
