// The user's side of a chat message (#8339): which avatar the bubble picks,
// and what it falls back to when `whoami` has no answer for it.
//
// `MessageBubble` is exported so this can be tested without mounting
// `ChatPage`. What is under test is the *wiring*, not the chain: that the image
// is what the bubble draws, then the emoji, then the initials, is pinned in
// `components/ui/Avatar.test.tsx`, and that the bytes come from the literal
// `me` path is pinned in `lib/queries/users-avatar.test.tsx`.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MessageBubble } from "./ChatPage";
import * as http from "../lib/http/client";

// `t` is called with a key and sometimes a fallback; the keys are enough for the
// assertions below. `importActual` for the rest of the module: `ChatPage` imports
// `initReactI18next` from here at module load, and a mock that omitted it would
// throw before a single test ran.
const translation = { t: (key: string) => key };
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return { ...actual, useTranslation: () => translation };
});

// The math-plugin load and the markdown renderer pull in katex and the whole
// markdown pipeline. Neither is what this file is about.
vi.mock("../lib/hooks/useMathPlugins", () => ({
  useMathPlugins: () => ({ remarkPlugins: [], rehypePlugins: [] }),
}));
vi.mock("../components/ui/MarkdownContent", () => ({
  MarkdownContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

// The user's avatar is fetched as bytes behind the bearer token. The default
// below is the 404 the daemon sends for a caller who has never uploaded one,
// which is the state every test here starts in except where noted.
vi.mock("../lib/http/client", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/http/client")>("../lib/http/client");
  return { ...actual, fetchAuthenticatedImage: vi.fn() };
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(http.fetchAuthenticatedImage).mockRejectedValue(new Error("404"));
});

type BubbleProps = React.ComponentProps<typeof MessageBubble>;

const MESSAGE = {
  id: "m1",
  role: "user",
  content: "hola",
  timestamp: new Date("2026-01-01T00:00:00Z"),
} satisfies BubbleProps["message"];

function renderBubble(props: Pick<BubbleProps, "userName" | "userEmoji">) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MessageBubble message={MESSAGE} usageFooter="" {...props} />
    </QueryClientProvider>,
  );
}

describe("the user's side of a message bubble", () => {
  it("draws the caller's avatar with their emoji", async () => {
    renderBubble({ userName: "alice", userEmoji: "🦊" });

    // The emoji is what `whoami` answers with, and it is what the circle shows
    // until an image arrives — so this also proves the identity reaches the
    // circle rather than being held one component up.
    const avatar = await screen.findByRole("img", { name: "alice" });
    expect(avatar).toHaveTextContent("🦊");
  });

  it("falls back to the initials when the caller has neither picture nor emoji", async () => {
    renderBubble({ userName: "jane doe" });

    const avatar = await screen.findByRole("img", { name: "jane doe" });
    expect(avatar).toHaveTextContent("JD");
  });

  it("keeps the generic person icon when there is nobody to name", () => {
    renderBubble({});

    // No name means no `UserAvatar` at all — that is the no-auth case and the
    // window before `whoami` resolves. Drawing an avatar from an empty name
    // would render a `?` where the icon belongs.
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    // The bubble itself is still there, so the assertion above is about the
    // avatar and not about a component that failed to render.
    expect(screen.getByText("chat.you")).toBeInTheDocument();
  });

  it("does not put the caller's identity on the agent's side", () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <MessageBubble
          message={{ ...MESSAGE, role: "assistant" }}
          usageFooter=""
          userName="alice"
          userEmoji="🦊"
        />
      </QueryClientProvider>,
    );

    expect(screen.queryByRole("img", { name: "alice" })).not.toBeInTheDocument();
    expect(screen.getByText("chat.bot")).toBeInTheDocument();
  });
});
