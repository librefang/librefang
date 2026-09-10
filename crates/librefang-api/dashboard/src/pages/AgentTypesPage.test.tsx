import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { AgentTypesPage } from "./AgentTypesPage";
import { useAgentType, useAgentTypes, useAgentTypeHistory } from "../lib/queries/agentTypes";
import { useAgents, useTools } from "../lib/queries/agents";
import { useSkills } from "../lib/queries/skills";
import {
  useDeleteAgentType,
  usePromoteAgentType,
  useRestoreTemplateVersion,
  useSpawnEphemeral,
} from "../lib/mutations/agentTypes";
import * as agentTypeMutations from "../lib/mutations/agentTypes";
import { ApiError } from "../lib/http/errors";
import { useUIStore } from "../lib/store";
import { createTestQueryClient } from "../lib/test/query-client";
import type { AgentTemplate, AgentTypeDetail } from "../api";

// The promotion flow (#7771) is the part of this page with no net: it opens a
// pull request against a public registry, so a control that fires the wrong
// mutation or swallows the server's refusal is expensive to discover in
// production.

vi.mock("../lib/queries/agentTypes", () => ({
  useAgentTypes: vi.fn(),
  useAgentType: vi.fn(),
  useAgentTypeHistory: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  useAgents: vi.fn(),
  useTools: vi.fn(),
}));

vi.mock("../lib/queries/skills", () => ({ useSkills: vi.fn() }));

// Both names for the manifest-write and manifest-create hooks: `main` exports
// them as `useUpdateAgentType` / `useCreateAgentType` and #8028 renames them to
// `useUpdateAgentTypeToml` / `useCreateAgentTypeFromToml`.
// This test only needs them stubbed — it never asserts on either — so the
// factory provides both spellings and the page gets whichever one it imports.
// Pinning a single name would break this file on whichever of the two PRs
// merges second, for hooks that have nothing to do with what is being tested.
vi.mock("../lib/mutations/agentTypes", () => ({
  useCreateAgentType: vi.fn(),
  useCreateAgentTypeFromToml: vi.fn(),
  useDeleteAgentType: vi.fn(),
  usePromoteAgentType: vi.fn(),
  useRestoreTemplateVersion: vi.fn(),
  useSpawnEphemeral: vi.fn(),
  useUpdateAgentType: vi.fn(),
  useUpdateAgentTypeToml: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children: React.ReactNode } & Record<string, unknown>) => (
    <a {...(rest as Record<string, unknown>)}>{children}</a>
  ),
}));

// motion/react drives Modal and ConfirmDialog through async animation hooks
// that don't settle in jsdom. Stub them so render is synchronous.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get:
        (_target, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

// Resolve keys against the real `en.json` rather than echoing them back.
// Asserting on the rendered English is what ties this file to #8166: a locale
// that declares `agentTypes.promote` twice again, with the other copy winning,
// changes these strings and fails here.
// Deliberately narrower than i18next: it interpolates only `{{name}}` with no
// inner spaces, does not do CLDR plural selection, and `useTranslation()` returns
// `t` without the `i18n` object.
// None of that reaches this page — it uses no `count` key and destructures only
// `{ t }` — but a future test rendering a component that calls `t(key, { count })`
// or reads `i18n.language` will fail here in a confusing way. Widen the mock then.
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  const en = (await import("../locales/en.json")).default as Record<string, unknown>;
  const lookup = (key: string): unknown =>
    key
      .split(".")
      .reduce<unknown>(
        (node, part) =>
          node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined,
        en,
      );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        const hit = lookup(key);
        const template =
          typeof hit === "string" ? hit : ((opts?.defaultValue as string | undefined) ?? key);
        return template.replace(/\{\{(\w+)\}\}/g, (_m, name: string) => String(opts?.[name] ?? ""));
      },
    }),
  };
});

const PROMOTE_LABEL = "Promote to registry";
const PREVIEW_LABEL = "Promotion preview";

const TYPE: AgentTemplate = {
  name: "researcher",
  description: "Reads papers",
  provider: "anthropic",
  model: "claude-sonnet-5",
  source: "agent-type",
  editable: true,
};

const DETAIL: AgentTypeDetail = {
  name: "researcher",
  source: "agent-type",
  editable: true,
  spec: { description: "Reads papers" },
  manifest_toml: 'name = "researcher"\n',
  promotion_preview: {
    requires_review: true,
    findings: [
      {
        field: "system_prompt",
        category: "path",
        preview: "/home/paco/notes",
        removed_by_sanitizer: false,
      },
    ],
    manifest_toml: 'name = "researcher"\ndescription = "Reads papers"\n',
  },
};

const idle = { mutateAsync: vi.fn(), isPending: false };

function mockQuery<T>(data: T) {
  return {
    data,
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  };
}

/** The promote mutation is the only one a test ever varies. */
function renderPage(promote: { mutateAsync: ReturnType<typeof vi.fn>; isPending: boolean }) {
  vi.mocked(useAgentTypes).mockReturnValue(
    mockQuery([TYPE]) as unknown as ReturnType<typeof useAgentTypes>,
  );
  vi.mocked(useAgentType).mockReturnValue(
    mockQuery(DETAIL) as unknown as ReturnType<typeof useAgentType>,
  );
  vi.mocked(useAgentTypeHistory).mockReturnValue(
    mockQuery({ versions: [] }) as unknown as ReturnType<typeof useAgentTypeHistory>,
  );
  vi.mocked(useAgents).mockReturnValue(mockQuery([]) as unknown as ReturnType<typeof useAgents>);
  vi.mocked(useTools).mockReturnValue(mockQuery([]) as unknown as ReturnType<typeof useTools>);
  vi.mocked(useSkills).mockReturnValue(mockQuery([]) as unknown as ReturnType<typeof useSkills>);
  // Stub both spellings of the manifest-write and manifest-create hooks rather
  // than picking one: the page calls whichever it imports, and an unstubbed
  // `vi.fn()` returns `undefined`, which the page then destructures and
  // crashes on.
  const mutations = agentTypeMutations as unknown as Record<string, unknown>;
  for (const hook of [
    useDeleteAgentType,
    useRestoreTemplateVersion,
    useSpawnEphemeral,
    mutations.useCreateAgentType,
    mutations.useCreateAgentTypeFromToml,
    mutations.useUpdateAgentType,
    mutations.useUpdateAgentTypeToml,
  ]) {
    if (!hook) continue;
    (hook as unknown as ReturnType<typeof vi.fn>).mockReturnValue(idle);
  }
  vi.mocked(usePromoteAgentType).mockReturnValue(
    promote as unknown as ReturnType<typeof usePromoteAgentType>,
  );

  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <AgentTypesPage />
    </QueryClientProvider>,
  );
}

// The two promotion controls are adjacent icon-only buttons with very
// different consequences — the first opens a read-only sanitized-manifest
// modal, the second opens a public registry pull request — so each is found by
// its own accessible name rather than by DOM position. They shared one until
// #8166: `agentTypes.promote` was defined twice in every locale, once per
// button, and last-wins silently relabelled the preview.
function previewButton() {
  return screen.getByRole("button", { name: PREVIEW_LABEL });
}

function promoteButton() {
  return screen.getByRole("button", { name: PROMOTE_LABEL });
}

describe("AgentTypesPage promotion", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUIStore.setState({ toasts: [] });
  });

  // Icon-only controls, so the accessible name is the only thing telling a
  // screen-reader user which one publishes. `getByRole` throws on more than one
  // match, so each of these also asserts the other button did not borrow the
  // name (#8166).
  it("gives the preview and publish controls distinct accessible names", () => {
    renderPage({ mutateAsync: vi.fn(), isPending: false });

    expect(previewButton()).toBeInTheDocument();
    expect(promoteButton()).toBeInTheDocument();
    expect(previewButton()).not.toBe(promoteButton());
    // Both surfaces of the name, since the sighted user reads the tooltip.
    expect(previewButton()).toHaveAttribute("title", PREVIEW_LABEL);
    expect(promoteButton()).toHaveAttribute("title", PROMOTE_LABEL);
  });

  it("opens the sanitized manifest and its retained findings from the preview button", () => {
    renderPage({ mutateAsync: vi.fn(), isPending: false });
    fireEvent.click(previewButton());

    expect(screen.getByText(/Sanitized manifest/)).toBeInTheDocument();
    expect(screen.getByText(/description = "Reads papers"/)).toBeInTheDocument();
    // A finding the sanitizer does NOT strip has to read as the operator's
    // problem, not as a note — that is the whole point of the preview.
    expect(screen.getByText("system_prompt")).toBeInTheDocument();
    expect(screen.getByText("Needs review")).toBeInTheDocument();
    expect(
      screen.getByText(/still has values to check by hand/),
    ).toBeInTheDocument();
  });

  it("confirms before promoting, then shows the pull request it opened", async () => {
    const mutateAsync = vi.fn().mockResolvedValue({ pr_url: "https://example.test/pr/7" });
    renderPage({ mutateAsync, isPending: false });

    fireEvent.click(promoteButton());
    expect(screen.getByText(/Promote the agent type 'researcher'/)).toBeInTheDocument();
    expect(mutateAsync).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith("researcher"));
    const link = await screen.findByRole("link", { name: /View pull request/ });
    expect(link).toHaveAttribute("href", "https://example.test/pr/7");
    expect(useUIStore.getState().toasts.map((t) => t.message)).toContain("Registry PR opened");
  });

  it("surfaces a 409 review_required as the server's reason, not a generic failure", async () => {
    const reason = "Manifest retains 1 finding that needs review before publication";
    const mutateAsync = vi
      .fn()
      .mockRejectedValue(new ApiError(409, "review_required", reason));
    renderPage({ mutateAsync, isPending: false });

    fireEvent.click(promoteButton());
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() => expect(useUIStore.getState().toasts).toHaveLength(1));
    const toast = useUIStore.getState().toasts[0];
    expect(toast.type).toBe("error");
    expect(toast.message).toContain("409");
    expect(toast.message).toContain(reason);
    // The generic fallback is what the operator gets when the reason is lost.
    expect(toast.message).not.toBe("Could not promote the agent type");
    // A refused promotion has opened no pull request, so the success dialog
    // must stay closed.
    expect(screen.queryByRole("link", { name: /View pull request/ })).toBeNull();
  });
});
