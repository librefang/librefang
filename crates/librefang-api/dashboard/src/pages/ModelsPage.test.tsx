import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ModelsPage } from "./ModelsPage";
import { useModels, useModelOverrides } from "../lib/queries/models";
import {
  useAddCustomModel,
  useRemoveCustomModel,
  useUpdateModelOverrides,
  useDeleteModelOverrides,
} from "../lib/mutations/models";
import type { ModelItem } from "../api";
import { useUIStore } from "../lib/store";

vi.mock("../lib/queries/models", () => ({
  useModels: vi.fn(),
  useModelOverrides: vi.fn(),
}));

vi.mock("../lib/mutations/models", () => ({
  useAddCustomModel: vi.fn(),
  useRemoveCustomModel: vi.fn(),
  useUpdateModelOverrides: vi.fn(),
  useDeleteModelOverrides: vi.fn(),
}));

// DrawerPanel pushes its children into a global slot via Zustand instead of
// rendering inline, so jsdom queries for form fields inside the drawer would
// miss them. Replace it with a passthrough that renders children when open.
vi.mock("../components/ui/DrawerPanel", () => ({
  DrawerPanel: ({ isOpen, title, children }: { isOpen: boolean; title?: string; children: React.ReactNode }) =>
    isOpen ? (
      <div data-testid="drawer-panel">
        {title && <div data-testid="drawer-title">{title}</div>}
        {children}
      </div>
    ) : null,
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({ t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key }),
  };
});

const useModelsMock = useModels as unknown as ReturnType<typeof vi.fn>;
const useModelOverridesMock = useModelOverrides as unknown as ReturnType<typeof vi.fn>;
const useAddCustomModelMock = useAddCustomModel as unknown as ReturnType<typeof vi.fn>;
const useRemoveCustomModelMock = useRemoveCustomModel as unknown as ReturnType<typeof vi.fn>;
const useUpdateModelOverridesMock = useUpdateModelOverrides as unknown as ReturnType<typeof vi.fn>;
const useDeleteModelOverridesMock = useDeleteModelOverrides as unknown as ReturnType<typeof vi.fn>;

interface QueryShape<T> {
  data: T;
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  refetch: ReturnType<typeof vi.fn>;
}

function makeQuery<T>(data: T, overrides: Partial<QueryShape<T>> = {}): QueryShape<T> {
  return {
    data,
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

interface MutShape {
  mutate: ReturnType<typeof vi.fn>;
  mutateAsync: ReturnType<typeof vi.fn>;
  isPending: boolean;
  error: Error | null;
}

function makeMut(overrides: Partial<MutShape> = {}): MutShape {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn().mockResolvedValue(undefined),
    isPending: false,
    error: null,
    ...overrides,
  };
}

const sampleModels: ModelItem[] = [
  {
    id: "gpt-4o",
    display_name: "GPT-4o",
    provider: "openai",
    tier: "frontier",
    context_window: 128000,
    input_cost_per_m: 2.5,
    output_cost_per_m: 10,
    supports_tools: true,
    supports_vision: true,
    supports_streaming: true,
    available: true,
  },
  {
    id: "claude-haiku",
    display_name: "Claude Haiku",
    provider: "anthropic",
    tier: "fast",
    context_window: 200000,
    input_cost_per_m: 1,
    output_cost_per_m: 5,
    supports_tools: true,
    available: true,
  },
  {
    id: "my-custom",
    display_name: "My Custom Model",
    provider: "openai",
    tier: "custom",
    context_window: 32000,
    input_cost_per_m: 0,
    output_cost_per_m: 0,
    available: false,
  },
];

function setLoaded(models: ModelItem[] = sampleModels): void {
  useModelsMock.mockReturnValue(
    makeQuery({ models, total: models.length, available: models.filter(m => m.available).length }),
  );
}

function setMutationDefaults(): {
  add: MutShape;
  remove: MutShape;
  update: MutShape;
  del: MutShape;
} {
  const add = makeMut();
  const remove = makeMut();
  const update = makeMut();
  const del = makeMut();
  useAddCustomModelMock.mockReturnValue(add);
  useRemoveCustomModelMock.mockReturnValue(remove);
  useUpdateModelOverridesMock.mockReturnValue(update);
  useDeleteModelOverridesMock.mockReturnValue(del);
  return { add, remove, update, del };
}

function renderPage(): void {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <ModelsPage />
    </QueryClientProvider>,
  );
}

describe("ModelsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Reset persisted Zustand state that affects filtering visibility.
    useUIStore.setState({
      hiddenModelKeys: [],
      modelsAvailableOnly: false,
      toasts: [],
    });
    useModelOverridesMock.mockReturnValue(makeQuery({}));
    setMutationDefaults();
  });

  it("shows the load-error banner when the models query errors", () => {
    useModelsMock.mockReturnValue(makeQuery(undefined, { isError: true }));
    renderPage();
    expect(screen.getByText("models.load_error")).toBeInTheDocument();
  });

  it("shows ListSkeleton placeholder while models query is loading", () => {
    useModelsMock.mockReturnValue(makeQuery(undefined, { isLoading: true }));
    renderPage();
    // No model cards; empty-state copy not rendered either.
    expect(screen.queryByText("models.no_models")).not.toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("shows the empty-state when the catalog is empty", () => {
    setLoaded([]);
    renderPage();
    expect(screen.getByText("models.no_models")).toBeInTheDocument();
  });

  it("renders model cards grouped by provider", () => {
    setLoaded();
    renderPage();
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    // Provider headers render.
    // Provider header text appears as a section header (and may also appear
    // in card subtitles like "openai/gpt-4o" — getAllByText keeps the test
    // tolerant to that).
    expect(screen.getAllByText("openai").length).toBeGreaterThan(0);
    expect(screen.getAllByText("anthropic").length).toBeGreaterThan(0);
  });

  it("filters by search query across id/display_name/provider", () => {
    setLoaded();
    renderPage();
    const search = screen.getByPlaceholderText("models.search_placeholder");
    fireEvent.change(search, { target: { value: "haiku" } });
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("filters by provider via the provider <select>", () => {
    setLoaded();
    renderPage();
    // First select after search input is providerFilter.
    const selects = screen.getAllByRole("combobox");
    fireEvent.change(selects[0], { target: { value: "anthropic" } });
    expect(screen.getByText("Claude Haiku")).toBeInTheDocument();
    expect(screen.queryByText("GPT-4o")).not.toBeInTheDocument();
  });

  it("hides custom-tier model when availableOnly toggle is on (it has available=false)", () => {
    setLoaded();
    renderPage();
    // Custom model is available=false, so it should be visible by default
    // (toggle off in beforeEach), then disappear after toggling.
    expect(screen.getByText("My Custom Model")).toBeInTheDocument();
    fireEvent.click(screen.getByTitle("models.available_only"));
    expect(screen.queryByText("My Custom Model")).not.toBeInTheDocument();
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
  });

  it("renders Free badge only when both costs are explicitly 0 (custom model)", () => {
    setLoaded();
    renderPage();
    // Custom model has both costs = 0, so Free badge appears.
    expect(screen.getByText("models.free")).toBeInTheDocument();
  });

  it("opens the Add Custom Model drawer when the header button is clicked", () => {
    setLoaded();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));
    expect(screen.getByText("models.add_custom_model")).toBeInTheDocument();
    // Required form fields render.
    expect(screen.getByPlaceholderText("models.model_id_placeholder")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("models.provider_placeholder")).toBeInTheDocument();
  });

  it("submits the Add form and calls useAddCustomModel.mutateAsync with trimmed payload", async () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));

    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "  new-model  " },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: " custom-provider " },
    });

    // Submit — the drawer contains a <form>, find it and dispatch submit.
    const form = screen.getByPlaceholderText("models.model_id_placeholder").closest("form");
    expect(form).not.toBeNull();
    fireEvent.submit(form!);

    expect(muts.add.mutateAsync).toHaveBeenCalledTimes(1);
    const payload = muts.add.mutateAsync.mock.calls[0][0];
    expect(payload.id).toBe("new-model");
    expect(payload.provider).toBe("custom-provider");
  });

  it("requires double-click to delete a custom model (confirm-then-delete)", () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();

    // Find the delete button for the custom model. It only appears on the
    // custom card; query by its title on either button.
    const deleteBtn = screen.getByTitle("models.delete_model");
    fireEvent.click(deleteBtn);
    // First click is just confirmation arming — should NOT call mutateAsync.
    expect(muts.remove.mutateAsync).not.toHaveBeenCalled();

    fireEvent.click(deleteBtn);
    expect(muts.remove.mutateAsync).toHaveBeenCalledTimes(1);
    expect(muts.remove.mutateAsync).toHaveBeenCalledWith("my-custom");
  });

  it("invokes refetch when the header refresh button fires", () => {
    const refetch = vi.fn().mockResolvedValue(undefined);
    useModelsMock.mockReturnValue(
      makeQuery(
        { models: sampleModels, total: 3, available: 2 },
        { refetch },
      ),
    );
    renderPage();
    fireEvent.click(screen.getByLabelText("common.refresh"));
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("toggles a model to hidden via the per-card hide button and updates the hidden filter chip", () => {
    setLoaded();
    renderPage();
    // The hide button is hover-revealed but still in the DOM.
    const hideBtns = screen.getAllByTitle("models.hide_model");
    expect(hideBtns.length).toBeGreaterThan(0);
    fireEvent.click(hideBtns[0]);
    // After hiding, the persisted store should record the key, and the
    // hidden-toggle chip with count appears.
    expect(useUIStore.getState().hiddenModelKeys.length).toBe(1);
    expect(screen.getByTitle("models.show_hidden")).toBeInTheDocument();
  });
});
