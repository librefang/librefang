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
