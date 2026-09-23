// Tests RestoreDiffModal directly, in its own file rather than inside
// AgentTypesPage.test.tsx: that file mocks the agentTypes query and mutation
// modules for the promotion flow (#7771), and `vi.mock` factories are
// per-file, so the two suites cannot share one without each stubbing hooks the
// other needs.

import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { RestoreDiffModal } from "./AgentTypesPage";
import { useAgentTypeRegistryDiff } from "../lib/queries/agentTypes";
import { useRestoreAgentType } from "../lib/mutations/agentTypes";
import { ApiError } from "../lib/http/errors";

vi.mock("../lib/queries/agentTypes", () => ({
  useAgentTypeRegistryDiff: vi.fn(),
}));

vi.mock("../lib/mutations/agentTypes", () => ({
  useRestoreAgentType: vi.fn(),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) =>
      opts && typeof opts === "object" && "defaultValue" in (opts as Record<string, unknown>)
        ? (opts as { defaultValue: string }).defaultValue
        : key,
    i18n: { language: "en" },
  }),
}));

const useAgentTypeRegistryDiffMock = useAgentTypeRegistryDiff as unknown as ReturnType<typeof vi.fn>;
const useRestoreAgentTypeMock = useRestoreAgentType as unknown as ReturnType<typeof vi.fn>;

describe("RestoreDiffModal error branch (#8042)", () => {
  it("keeps the registry-specific copy when the diff 404s with registry_type_not_found", () => {
    useAgentTypeRegistryDiffMock.mockReturnValue({
      isLoading: false,
      isError: true,
      error: new ApiError(404, "registry_type_not_found", "not in registry"),
      data: undefined,
    });
    useRestoreAgentTypeMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });

    render(<RestoreDiffModal name="local-only-type" onClose={() => {}} />);

    expect(screen.getByText("agentTypes.restore_no_registry")).toBeInTheDocument();
  });

  it("falls back to the server's own message for any other failure code, instead of always claiming the type is unregistered", () => {
    useAgentTypeRegistryDiffMock.mockReturnValue({
      isLoading: false,
      isError: true,
      error: new ApiError(500, "template_invalid_manifest", "local manifest is unparseable"),
      data: undefined,
    });
    useRestoreAgentTypeMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });

    render(<RestoreDiffModal name="broken-local-type" onClose={() => {}} />);

    expect(screen.queryByText("agentTypes.restore_no_registry")).not.toBeInTheDocument();
    expect(screen.getByText(/local manifest is unparseable/)).toBeInTheDocument();
  });
});

describe("RestoreDiffModal value rendering (#8054)", () => {
  // Six of the twelve fields the backend diffs are `Vec<String>`, so the array
  // case is the common one in this table rather than an edge case, and the
  // cells it renders into are `max-w-[200px] truncate`.
  it("renders a list field as a readable list, not as JSON", () => {
    useAgentTypeRegistryDiffMock.mockReturnValue({
      isLoading: false,
      isError: false,
      error: null,
      data: {
        name: "t",
        identical: false,
        unlisted_diffs: 0,
        diffs: [{ field: "tools", local: ["read_file"], registry: ["read_file", "write_file"] }],
        local_toml: "",
        registry_toml: "",
      },
    });
    useRestoreAgentTypeMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });

    render(<RestoreDiffModal name="t" onClose={() => {}} />);

    expect(screen.getByText("read_file, write_file")).toBeInTheDocument();
    expect(screen.queryByText('["read_file","write_file"]')).not.toBeInTheDocument();
  });

  // An unset `provider`/`model` arrives as JSON null. Rendering the literal
  // word `null` reads as a value the operator chose rather than one that is
  // absent, and the table already spells "nothing" as an em dash for the
  // empty-list case.
  it("renders an absent value as an em dash rather than the word null", () => {
    useAgentTypeRegistryDiffMock.mockReturnValue({
      isLoading: false,
      isError: false,
      error: null,
      data: {
        name: "t",
        identical: false,
        unlisted_diffs: 0,
        diffs: [
          { field: "provider", local: null, registry: "anthropic" },
          { field: "skills", local: [], registry: ["triage"] },
        ],
        local_toml: "",
        registry_toml: "",
      },
    });
    useRestoreAgentTypeMock.mockReturnValue({ mutateAsync: vi.fn(), isPending: false });

    render(<RestoreDiffModal name="t" onClose={() => {}} />);

    expect(screen.queryByText("null")).not.toBeInTheDocument();
    expect(screen.getAllByText("—")).toHaveLength(2);
  });
});
