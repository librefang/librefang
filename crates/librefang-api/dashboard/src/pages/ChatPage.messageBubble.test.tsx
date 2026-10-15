// #8371 review follow-up: the chat transcript resolves the selected agent's
// avatar once in `ChatPage` and hands the object URL to every `MessageBubble`.
// `AgentAvatar.test.tsx` covers the component alone; this renders the real
// bubble, so a dropped `agentAvatarSrc` passthrough — or a bubble that starts
// fetching for itself — fails somewhere instead of only breaking in the SPA.

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MessageBubble } from "./ChatPage";
import { fetchAuthenticatedImage } from "../lib/http/client";
import { createQueryClientWrapper } from "../lib/test/query-client";

// The object has to be referentially stable, the way `i18n.ts` keeps it: the
// bubble passes `t` down and the existing ChatPage tests mock it for the same
// reason. Partial, because `lib/i18n.ts` (pulled in through `lib/store`) calls
// `initReactI18next` at module scope.
const { translation } = vi.hoisted(() => ({
  translation: {
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
  },
}));

vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => translation,
}));

vi.mock("../lib/http/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/http/client")>()),
  fetchAuthenticatedImage: vi.fn(),
}));

const AGENT = "agent-1";
const RESOLVED = "blob:chat/1";

function assistantMessage(id: string, content: string) {
  return {
    id,
    role: "assistant" as const,
    content,
    timestamp: new Date("2026-10-03T00:00:00Z"),
  };
}

describe("MessageBubble's resolved avatar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("draws the URL ChatPage resolved once, without fetching per bubble", () => {
    const { wrapper } = createQueryClientWrapper();
    render(
      <>
        <MessageBubble
          message={assistantMessage("m1", "one")}
          usageFooter=""
          agentId={AGENT}
          agentName="Jane Doe"
          agentAvatarSrc={RESOLVED}
        />
        <MessageBubble
          message={assistantMessage("m2", "two")}
          usageFooter=""
          agentId={AGENT}
          agentName="Jane Doe"
          agentAvatarSrc={RESOLVED}
        />
      </>,
      { wrapper },
    );

    const avatars = screen.getAllByRole("img", { name: "Jane Doe" });
    expect(avatars).toHaveLength(2);
    for (const avatar of avatars) {
      // Both bubbles render the caller's object URL rather than minting (or
      // requesting bytes for) one of their own.
      expect(avatar.querySelector("img")).toHaveAttribute("src", RESOLVED);
    }
    expect(vi.mocked(fetchAuthenticatedImage)).not.toHaveBeenCalled();
  });
});
