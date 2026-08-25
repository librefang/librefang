import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PendingSkillsSection } from "./PendingSkillsSection";
import {
  usePendingSkillCandidates,
  useSkillDetail,
} from "../lib/queries/skills";
import {
  useApprovePendingCandidate,
  useProposePendingToRegistry,
  useRejectPendingCandidate,
} from "../lib/mutations/skills";
import type { PendingCandidate } from "../api";

vi.mock("../lib/queries/skills", () => ({
  usePendingSkillCandidates: vi.fn(),
  useSkillDetail: vi.fn(),
}));

vi.mock("../lib/mutations/skills", () => ({
  useApprovePendingCandidate: vi.fn(),
  useRejectPendingCandidate: vi.fn(),
  useProposePendingToRegistry: vi.fn(),
}));

vi.mock("./ui/ConfirmDialog", () => ({
  ConfirmDialog: ({ isOpen, message }: { isOpen: boolean; message: string }) =>
    isOpen ? <div role="dialog">{message}</div> : null,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string | Record<string, unknown>) => {
      if (typeof fallback === "string") return fallback;
      const defaultValue = String(fallback?.defaultValue ?? key);
      return defaultValue.replace("{{name}}", String(fallback?.name ?? ""));
    },
  }),
}));

const pendingMock = usePendingSkillCandidates as unknown as ReturnType<typeof vi.fn>;
const detailMock = useSkillDetail as unknown as ReturnType<typeof vi.fn>;
const approveMock = useApprovePendingCandidate as unknown as ReturnType<typeof vi.fn>;
const rejectMock = useRejectPendingCandidate as unknown as ReturnType<typeof vi.fn>;
const proposeMock = useProposePendingToRegistry as unknown as ReturnType<typeof vi.fn>;

function candidate(overrides: Partial<PendingCandidate> = {}): PendingCandidate {
  return {
    id: "candidate-1",
    agent_id: "11111111-1111-1111-1111-111111111111",
    captured_at: "2026-08-15T00:00:00Z",
    source: { kind: "explicit_instruction", trigger: "please remember" },
    name: "research-helper",
    description: "Researches a topic",
    prompt_context: "Use primary sources.",
    provenance: {
      user_message_excerpt: "Research this",
      turn_index: 1,
    },
    ...overrides,
  };
}

describe("PendingSkillsSection", () => {
  beforeEach(() => {
    detailMock.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: false,
    });
    approveMock.mockReturnValue({
      isPending: false,
      isError: false,
      mutate: vi.fn(),
    });
    rejectMock.mockReturnValue({
      isPending: false,
      isError: false,
      mutateAsync: vi.fn(),
    });
    proposeMock.mockReturnValue({
      isPending: false,
      isError: false,
      isSuccess: false,
      data: null,
      mutate: vi.fn(),
    });
  });

  it("degrades safely for an unknown capture source", () => {
    pendingMock.mockReturnValue({
      data: [candidate({
        source: { kind: "future_source" } as unknown as PendingCandidate["source"],
      })],
      isLoading: false,
      isError: false,
    });

    render(<PendingSkillsSection />);
    expect(screen.getByText("Unknown source")).toBeInTheDocument();
    expect(screen.getByText("future_source")).toBeInTheDocument();
  });

  it("blocks an incomplete update instead of diffing against an empty body", () => {
    pendingMock.mockReturnValue({
      data: [candidate({ kind: "update", current_version: "1.0.0" })],
      isLoading: false,
      isError: false,
    });

    render(<PendingSkillsSection />);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "This update is missing its target skill",
    );
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Propose to Registry" })).toBeDisabled();
    expect(detailMock).not.toHaveBeenCalled();
  });

  it("shows a rejection failure inside the open confirmation", () => {
    pendingMock.mockReturnValue({
      data: [candidate()],
      isLoading: false,
      isError: false,
    });
    rejectMock.mockReturnValue({
      isPending: false,
      isError: true,
      error: new Error("candidate is locked"),
      mutateAsync: vi.fn(),
    });

    render(<PendingSkillsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "Reject failed: candidate is locked",
    );
  });
});
