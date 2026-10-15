import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentAvatar } from "./AgentAvatar";
import { useAgentAvatarUrl } from "../lib/queries/agents";

vi.mock("../lib/queries/agents", () => ({
  useAgentAvatarUrl: vi.fn(),
}));

const mockAvatarUrl = vi.mocked(useAgentAvatarUrl);
const AVATAR = "/api/agents/a1/avatar";

describe("AgentAvatar", () => {
  beforeEach(() => {
    mockAvatarUrl.mockReset();
    // What the hook returns while the blob is still in flight, and when the
    // agent has no image at all. Both are `undefined` by design.
    mockAvatarUrl.mockReturnValue(undefined);
  });

  // The pair matters: the negative half only proves anything because the
  // positive half shows the assertion can fail. `hasAvatar` gates the request,
  // so an agent without an image must not cost a 404 on every render.
  it("asks for the image when the identity carries an avatar_url", () => {
    render(<AgentAvatar agentId="a1" avatarUrl={AVATAR} fallback="Jane Doe" />);

    expect(mockAvatarUrl).toHaveBeenCalledWith("a1", true);
  });

  it("does not ask for an image when the identity carries none", () => {
    render(<AgentAvatar agentId="a1" fallback="Jane Doe" />);

    expect(mockAvatarUrl).toHaveBeenCalledWith("a1", false);
  });

  it("renders the object URL the hook resolves", () => {
    mockAvatarUrl.mockReturnValue("blob:test/1");

    render(<AgentAvatar agentId="a1" avatarUrl={AVATAR} fallback="Jane Doe" />);

    expect(
      screen.getByRole("img", { name: "Jane Doe" }).querySelector("img"),
    ).toHaveAttribute("src", "blob:test/1");
  });

  it("uses a caller-resolved URL and asks the hook for nothing", () => {
    mockAvatarUrl.mockReturnValue("blob:own/9");

    render(
      <AgentAvatar
        agentId="a1"
        avatarUrl={AVATAR}
        resolvedSrc="blob:chat/1"
        fallback="Jane Doe"
      />,
    );

    // The chat transcript resolves the image once and hands the URL to every
    // bubble. Each bubble must render that shared URL rather than minting a
    // handle of its own, and must not leave the hook fetching behind it.
    expect(mockAvatarUrl).toHaveBeenCalledWith("a1", false);
    expect(
      screen.getByRole("img", { name: "Jane Doe" }).querySelector("img"),
    ).toHaveAttribute("src", "blob:chat/1");
  });

  it("shows the emoji while the image is still in flight", () => {
    // The hook resolves to `undefined` until the blob arrives. The identity
    // must not be blank for however long that takes.
    render(<AgentAvatar agentId="a1" avatarUrl={AVATAR} emoji="🤖" fallback="Jane Doe" />);

    expect(screen.getByRole("img", { name: "Jane Doe" })).toHaveTextContent("🤖");
  });

  it("falls back to the initials when there is neither image nor emoji", () => {
    render(<AgentAvatar agentId="a1" fallback="Jane Doe" />);

    expect(screen.getByRole("img", { name: "Jane Doe" })).toHaveTextContent("JD");
  });

  it("keeps the name as the accessible label rather than the initials", () => {
    render(<AgentAvatar agentId="a1" fallback="Jane Doe" />);

    expect(screen.getByRole("img")).toHaveAttribute("aria-label", "Jane Doe");
  });

  it("forwards className to the avatar beneath it", () => {
    // The chat's agent picker restates all three of its background states
    // through `className`. If this passthrough is ever dropped, those states
    // vanish and — without this test — every other test still passes.
    render(<AgentAvatar agentId="a1" fallback="Jane Doe" className="bg-white/20" />);

    const avatar = screen.getByRole("img", { name: "Jane Doe" });
    expect(avatar).toHaveClass("bg-white/20");
    expect(avatar).not.toHaveClass("bg-brand/10");
  });
});
