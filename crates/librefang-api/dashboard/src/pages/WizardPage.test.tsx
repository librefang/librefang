import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WizardPage } from "./WizardPage";
import { useProviders } from "../lib/queries/providers";
import {
  useSetDefaultProvider,
  useValidateProviderKey,
} from "../lib/mutations/providers";
import { useQuickInit } from "../lib/mutations/overview";
import { useUIStore } from "../lib/store";

vi.mock("../lib/queries/providers", () => ({ useProviders: vi.fn() }));
vi.mock("../lib/mutations/providers", () => ({
  useSetDefaultProvider: vi.fn(),
  useValidateProviderKey: vi.fn(),
}));
vi.mock("../lib/mutations/overview", () => ({ useQuickInit: vi.fn() }));
vi.mock("../lib/store", () => ({ useUIStore: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string) => key }),
  };
});

const mock = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;
const useProvidersMock = mock(useProviders);
const useSetDefaultProviderMock = mock(useSetDefaultProvider);
const useValidateProviderKeyMock = mock(useValidateProviderKey);
const useQuickInitMock = mock(useQuickInit);
const useUIStoreMock = mock(useUIStore);

const validate = vi.fn();
const setDefault = vi.fn();
const quickInit = vi.fn();
const addToast = vi.fn();

function mutation(mutateAsync: ReturnType<typeof vi.fn>) {
  return { mutateAsync, isPending: false, isError: false, error: null };
}

async function reachFinishStep() {
  fireEvent.click(screen.getByText("Groq").closest("button")!);
  fireEvent.click(screen.getByRole("button", { name: "wizard.connect" }));
  await screen.findByRole("button", { name: "wizard.finish_action" });
}

beforeEach(() => {
  vi.clearAllMocks();
  useProvidersMock.mockReturnValue({
    data: [
      {
        id: "groq",
        display_name: "Groq",
        key_required: false,
        auth_status: "not_required",
      },
    ],
    isLoading: false,
  });
  validate.mockResolvedValue(undefined);
  setDefault.mockResolvedValue(undefined);
  quickInit.mockResolvedValue(undefined);
  useValidateProviderKeyMock.mockReturnValue(mutation(validate));
  useSetDefaultProviderMock.mockReturnValue(mutation(setDefault));
  useQuickInitMock.mockReturnValue(mutation(quickInit));
  useUIStoreMock.mockImplementation(
    (selector: (state: { addToast: typeof addToast }) => unknown) =>
      selector({ addToast }),
  );
});

describe("WizardPage", () => {
  it("uses the provider validation mutation as the sole pending/error owner", async () => {
    render(<WizardPage />);
    fireEvent.click(screen.getByText("Groq").closest("button")!);
    fireEvent.click(screen.getByRole("button", { name: "wizard.connect" }));

    await waitFor(() => expect(validate).toHaveBeenCalledTimes(1));
    expect(validate).toHaveBeenCalledWith({ providerId: "groq", apiKey: "" });
    expect(
      await screen.findByRole("button", { name: "wizard.finish_action" }),
    ).toBeInTheDocument();
  });

  it("blocks rapid duplicate finalize calls with a synchronous lock", async () => {
    let resolveDefault!: () => void;
    setDefault.mockImplementation(
      () => new Promise<void>((resolve) => { resolveDefault = resolve; }),
    );
    render(<WizardPage />);
    await reachFinishStep();
    const finish = screen.getByRole("button", { name: "wizard.finish_action" });

    fireEvent.click(finish);
    fireEvent.click(finish);
    expect(setDefault).toHaveBeenCalledTimes(1);

    resolveDefault();
    await waitFor(() => expect(quickInit).toHaveBeenCalledTimes(1));
  });

  it("retries initialization without reapplying a saved default provider", async () => {
    quickInit
      .mockRejectedValueOnce(new Error("init unavailable"))
      .mockResolvedValueOnce(undefined);
    render(<WizardPage />);
    await reachFinishStep();

    fireEvent.click(screen.getByRole("button", { name: "wizard.finish_action" }));
    await screen.findByRole("button", { name: "common.retry" });
    expect(setDefault).toHaveBeenCalledTimes(1);
    expect(quickInit).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith("wizard.quick_init_failed", "error");

    fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
    await screen.findByText("wizard.done_title");
    expect(setDefault).toHaveBeenCalledTimes(1);
    expect(quickInit).toHaveBeenCalledTimes(2);
  });
});
