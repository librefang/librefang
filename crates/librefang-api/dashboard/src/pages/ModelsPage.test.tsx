import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  ModelsPage,
  capabilityOverrideLabel,
  modelPricingKind,
  numberInputValue,
  settingsStateEqual,
  settingsStateFromOverrides,
} from "./ModelsPage";
import { useModels, useModelOverrides } from "../lib/queries/models";
import {
  useAddCustomModel,
  useRemoveCustomModel,
  useUpdateModelOverrides,
  useDeleteModelOverrides,
} from "../lib/mutations/models";
import { useConfigStatus, useMediaModelEndpoints } from "../lib/queries/config";
import { useSaveMediaModelEndpoint } from "../lib/mutations/config";
import type { MediaModelEndpoint, ModelItem } from "../api";
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

vi.mock("../lib/queries/config", () => ({
  useMediaModelEndpoints: vi.fn(),
  useConfigStatus: vi.fn(),
}));

vi.mock("../lib/mutations/config", () => ({
  useSaveMediaModelEndpoint: vi.fn(),
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
const useMediaModelEndpointsMock = useMediaModelEndpoints as unknown as ReturnType<typeof vi.fn>;
const useConfigStatusMock = useConfigStatus as unknown as ReturnType<typeof vi.fn>;
const useSaveMediaModelEndpointMock = useSaveMediaModelEndpoint as unknown as ReturnType<typeof vi.fn>;

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

// Mirrors `selectMediaModelEndpoints` output for a config with a self-hosted
// Whisper for STT and a local llava for image description — `[media.custom_image]`
// is the *understanding* path (`describe_image`), not generation (refs #8038, #8011).
const sampleMediaEndpoints: MediaModelEndpoint[] = [
  {
    kind: "stt",
    config_path: "media.custom_stt",
    provider_path: "media.audio_provider",
    provider: "local-whisper",
    config: {
      base_url: "http://localhost:8080/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: false,
      model: "large-v3",
    },
    configured: true,
    modality_enabled: true,
    modality_enabled_path: "media.audio_transcription",
    model_override: null,
    model_override_path: "media.audio_model",
  },
  {
    kind: "tts",
    config_path: "tts.custom",
    provider_path: "tts.provider",
    provider: "local-piper",
    config: {
      base_url: "http://localhost:5000/v1/audio/speech",
      api_key_env: "",
      key_required: false,
      model: "tts-1",
      voice: "en_US-lessac-medium",
      format: "mp3",
    },
    configured: true,
    modality_enabled: true,
    modality_enabled_path: "tts.enabled",
    model_override: null,
    model_override_path: null,
  },
  {
    kind: "image",
    config_path: "media.custom_image",
    provider_path: "media.image_provider",
    provider: "local-llava",
    config: {
      base_url: "http://localhost:11434/v1/chat/completions",
      api_key_env: "",
      key_required: false,
      model: "llava",
    },
    configured: true,
    modality_enabled: true,
    modality_enabled_path: "media.image_description",
    model_override: null,
    model_override_path: "media.image_model",
  },
  {
    kind: "video",
    config_path: "media.custom_video",
    provider_path: "media.video_provider",
    provider: "",
    config: { base_url: "", api_key_env: "", key_required: false, model: null },
    configured: false,
    modality_enabled: false,
    modality_enabled_path: "media.video_description",
    model_override: null,
    model_override_path: "media.video_model",
  },
];

function setMediaEndpoints(
  endpoints: MediaModelEndpoint[] = sampleMediaEndpoints,
  overrides: Partial<QueryShape<MediaModelEndpoint[]>> = {},
): void {
  useMediaModelEndpointsMock.mockReturnValue(makeQuery(endpoints, overrides));
}

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

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
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
    setMediaEndpoints([]);
    useSaveMediaModelEndpointMock.mockReturnValue(makeMut());
    useConfigStatusMock.mockReturnValue(makeQuery({ writable: true, source: "" }));
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
    // Addressed by label, not by index: the filter bar gained a model-type
    // select in #8038 and an index would silently retarget.
    fireEvent.change(screen.getByLabelText("models.filter_provider"), {
      target: { value: "anthropic" },
    });
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

  it("does not label unknown OpenRouter pricing as free", () => {
    setLoaded([
      {
        id: "openrouter/acme/unknown",
        display_name: "Unknown pricing",
        provider: "openrouter",
        tier: "balanced",
        context_window: 32_768,
        input_cost_per_m: 0,
        output_cost_per_m: 0,
        pricing_known: false,
        available: true,
      },
    ]);
    setMutationDefaults();
    renderPage();

    expect(screen.queryByText("models.free")).toBeNull();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("treats missing costs as unknown pricing", () => {
    const model: ModelItem = {
      id: "missing-costs",
      display_name: "Missing costs",
      provider: "custom",
      context_window: 32_768,
      available: true,
    };
    setLoaded([model]);
    renderPage();

    expect(modelPricingKind(model)).toBe("unknown");
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.queryByText(/\$—/)).not.toBeInTheDocument();
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

  it("keeps cleared numeric fields empty and omits them on submit", () => {
    setLoaded();
    const muts = setMutationDefaults();
    renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));

    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "minimal-model" },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: "custom" },
    });
    for (const label of [
      "models.context_window",
      "models.max_output",
      "models.input_cost",
      "models.output_cost",
    ]) {
      const input = screen.getByLabelText(label) as HTMLInputElement;
      fireEvent.change(input, { target: { value: "" } });
      expect(input.value).toBe("");
    }

    const form = screen
      .getByPlaceholderText("models.model_id_placeholder")
      .closest("form");
    fireEvent.submit(form!);
    const payload = muts.add.mutateAsync.mock.calls[0][0];
    expect(payload).not.toHaveProperty("context_window");
    expect(payload).not.toHaveProperty("max_output_tokens");
    expect(payload).not.toHaveProperty("input_cost_per_m");
    expect(payload).not.toHaveProperty("output_cost_per_m");
  });

  it("does not apply add-model completion effects after unmount", async () => {
    setLoaded();
    let resolveAdd: () => void = () => undefined;
    const mutateAsync = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveAdd = resolve;
        }),
    );
    useAddCustomModelMock.mockReturnValue(makeMut({ mutateAsync }));
    const view = renderPage();
    fireEvent.click(screen.getByTitle("models.add_model (n)"));
    fireEvent.change(screen.getByPlaceholderText("models.model_id_placeholder"), {
      target: { value: "deferred-model" },
    });
    fireEvent.change(screen.getByPlaceholderText("models.provider_placeholder"), {
      target: { value: "custom" },
    });
    fireEvent.submit(
      screen.getByPlaceholderText("models.model_id_placeholder").closest("form")!,
    );

    view.unmount();
    resolveAdd();
    await act(async () => {
      await Promise.resolve();
    });
    expect(useUIStore.getState().toasts).toEqual([]);
  });

  it("resyncs pristine settings but preserves dirty settings", async () => {
    let overrides = { temperature: 0.2 };
    useModelOverridesMock.mockImplementation(() => makeQuery(overrides));
    setLoaded();
    renderPage();
    fireEvent.click(screen.getAllByTitle("models.settings_title")[0]);

    const temperatureRange = () =>
      screen
        .getAllByLabelText("models.temperature")
        .find(
          (element): element is HTMLInputElement =>
            element instanceof HTMLInputElement && element.type === "range",
        )!;
    await waitFor(() => expect(temperatureRange().value).toBe("0.2"));

    overrides = { temperature: 0.5 };
    fireEvent.change(screen.getByPlaceholderText("models.search_placeholder"), {
      target: { value: "g" },
    });
    await waitFor(() => expect(temperatureRange().value).toBe("0.5"));

    fireEvent.change(temperatureRange(), { target: { value: "0.9" } });
    overrides = { temperature: 0.7 };
    fireEvent.change(screen.getByPlaceholderText("models.search_placeholder"), {
      target: { value: "gp" },
    });
    await waitFor(() => expect(temperatureRange().value).toBe("0.9"));
  });

  it("does not apply settings-save effects after the drawer unmounts", async () => {
    let resolveSave: () => void = () => undefined;
    const mutateAsync = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveSave = resolve;
        }),
    );
    useUpdateModelOverridesMock.mockReturnValue(makeMut({ mutateAsync }));
    setLoaded();
    renderPage();
    fireEvent.click(screen.getAllByTitle("models.settings_title")[0]);
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    resolveSave();
    await act(async () => {
      await Promise.resolve();
    });
    expect(useUIStore.getState().toasts).toEqual([]);
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

  it("does not render a delete button for a cli_config-sourced model", () => {
    // A live-detected CLI model is tier "custom" but source "cli_config": it has
    // no persisted custom entry, so a delete would 404. The control must be hidden.
    setLoaded([
      {
        id: "codex-cli/deepseek-chat",
        display_name: "deepseek-chat (Codex CLI)",
        provider: "codex-cli",
        tier: "custom",
        source: "cli_config",
        context_window: 0,
        input_cost_per_m: 0,
        output_cost_per_m: 0,
        available: true,
      },
    ]);
    setMutationDefaults();
    renderPage();
    expect(screen.queryByTitle("models.delete_model")).toBeNull();
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

  // ── Media endpoints in the Models tab (refs #8038, #8011) ────────

  it("renders media endpoints alongside LLM models, each with a type label", () => {
    setLoaded();
    setMediaEndpoints();
    renderPage();

    // The LLM catalogue is still there …
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    // … and every media modality is now a row in the same tab.
    expect(screen.getByText("models.media_endpoints")).toBeInTheDocument();
    // One chip per model card, plus the entry in the type filter's option list.
    expect(screen.getAllByText("models.kind_llm").length).toBe(sampleModels.length + 1);
    for (const kind of ["stt", "tts", "image", "video"]) {
      expect(screen.getAllByText(`models.kind_${kind}`).length).toBeGreaterThan(0);
    }
    // Each card shows the endpoint it points at and the config path that owns it.
    expect(
      screen.getByText("http://localhost:8080/v1/audio/transcriptions"),
    ).toBeInTheDocument();
    expect(screen.getByText("media.custom_stt")).toBeInTheDocument();
    expect(screen.getByText("tts.custom")).toBeInTheDocument();
    // An unconfigured modality is still listed — that discoverability is the ask.
    expect(screen.getByText("models.media_not_configured")).toBeInTheDocument();
    expect(screen.getByText("models.media_provider_unset")).toBeInTheDocument();
  });

  it("filters the tab down to one modality via the type select", () => {
    setLoaded();
    setMediaEndpoints();
    renderPage();

    fireEvent.change(screen.getByLabelText("models.filter_kind"), {
      target: { value: "stt" },
    });

    expect(screen.getByText("media.custom_stt")).toBeInTheDocument();
    expect(screen.queryByText("tts.custom")).toBeNull();
    // LLM cards are gone, and the "no models" empty state must not take over.
    expect(screen.queryByText("GPT-4o")).toBeNull();
    expect(screen.queryByText("models.no_results")).toBeNull();

    fireEvent.change(screen.getByLabelText("models.filter_kind"), {
      target: { value: "llm" },
    });
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.queryByText("models.media_endpoints")).toBeNull();
  });

  it("matches media endpoints against the shared search box", () => {
    setLoaded();
    setMediaEndpoints();
    renderPage();

    fireEvent.change(screen.getByPlaceholderText("models.search_placeholder"), {
      target: { value: "whisper" },
    });

    expect(screen.getByText("media.custom_stt")).toBeInTheDocument();
    expect(screen.queryByText("media.custom_image")).toBeNull();
  });

  it("saves an edited media endpoint with the draft and the provider name", async () => {
    const save = makeMut();
    useSaveMediaModelEndpointMock.mockReturnValue(save);
    setLoaded([]);
    setMediaEndpoints([sampleMediaEndpoints[0]]);
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "models.kind_stt" }));
    fireEvent.change(screen.getByLabelText("models.media_base_url"), {
      target: { value: "http://whisper.internal/v1/audio/transcriptions" },
    });
    fireEvent.change(screen.getByLabelText("models.media_model"), {
      target: { value: "medium.en" },
    });
    fireEvent.change(screen.getByLabelText("models.media_provider"), {
      target: { value: "my-whisper" },
    });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => expect(save.mutateAsync).toHaveBeenCalledTimes(1));
    expect(save.mutateAsync).toHaveBeenCalledWith({
      endpoint: sampleMediaEndpoints[0],
      draft: {
        base_url: "http://whisper.internal/v1/audio/transcriptions",
        key_required: false,
        model: "medium.en",
      },
      provider: "my-whisper",
    });
    await waitFor(() =>
      expect(useUIStore.getState().toasts.map((toast) => toast.message)).toContain(
        "models.media_saved",
      ),
    );
  });

  it("exposes voice and format for TTS only", () => {
    setLoaded([]);
    setMediaEndpoints([sampleMediaEndpoints[1]]);
    const { unmount } = renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_tts" }));
    expect(screen.getByLabelText("models.media_voice")).toBeInTheDocument();
    expect(screen.getByLabelText("models.media_format")).toBeInTheDocument();
    unmount();

    setMediaEndpoints([sampleMediaEndpoints[2]]);
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_image" }));
    expect(screen.queryByLabelText("models.media_voice")).toBeNull();
    expect(screen.queryByLabelText("models.media_format")).toBeNull();
  });

  it("shows the API key env var read-only and never sends it from the form", () => {
    setLoaded([]);
    setMediaEndpoints([sampleMediaEndpoints[0]]);
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_stt" }));

    const envInput = screen.getByLabelText("models.media_api_key_env") as HTMLInputElement;
    // The env-var *name* is shown (never a key value), and it cannot be
    // repointed from here — that write stays an on-disk edit.
    expect(envInput.value).toBe("MY_LOCAL_WHISPER_KEY");
    expect(envInput.readOnly).toBe(true);
    expect(envInput.disabled).toBe(true);
  });

  it("still lists an unconfigured endpoint with the persisted default filters", () => {
    // `modelsAvailableOnly` defaults to `true` and is persisted (`lib/store.ts`),
    // so gating media rows on `configured` would hide all four on a fresh
    // install — the exact discoverability gap #8011 filed.
    useUIStore.setState({ modelsAvailableOnly: true });
    setLoaded();
    setMediaEndpoints();
    renderPage();

    expect(screen.getByText("models.media_endpoints")).toBeInTheDocument();
    expect(screen.getByText("media.custom_video")).toBeInTheDocument();
    expect(screen.getByText("models.media_not_configured")).toBeInTheDocument();
  });

  it("counts only the rows it actually renders", () => {
    setLoaded();
    setMediaEndpoints();
    renderPage();
    expect(
      screen.getByText(`${sampleModels.length + sampleMediaEndpoints.length} models.results`),
    ).toBeInTheDocument();

    // `filtered` is never narrowed by `kindFilter`, so the count has to drop
    // the LLM catalogue itself once its sections stop rendering.
    fireEvent.change(screen.getByLabelText("models.filter_kind"), {
      target: { value: "stt" },
    });
    expect(screen.getByText("1 models.results")).toBeInTheDocument();
  });

  it("suppresses media rows for filters that cannot match one", () => {
    setLoaded();
    setMediaEndpoints();
    renderPage();

    // Tiers are derived from the LLM catalogue, so no endpoint can match one.
    fireEvent.change(screen.getByLabelText("models.filter_tier"), {
      target: { value: sampleModels[0].tier as string },
    });
    expect(screen.queryByText("models.media_endpoints")).toBeNull();
    fireEvent.change(screen.getByLabelText("models.filter_tier"), {
      target: { value: "all" },
    });

    // A provider narrowed to an LLM vendor keeps only endpoints with that
    // provider name — none, here.
    fireEvent.change(screen.getByLabelText("models.filter_provider"), {
      target: { value: sampleModels[0].provider },
    });
    expect(screen.queryByText("models.media_endpoints")).toBeNull();
  });

  it("warns that a modality whose master switch is off is inert", () => {
    setLoaded([]);
    setMediaEndpoints([
      { ...sampleMediaEndpoints[1], modality_enabled: false, modality_enabled_path: "tts.enabled" },
    ]);
    renderPage();

    // `TtsConfig::default().enabled` is false, so a complete `[tts.custom]`
    // synthesises nothing until the flag is flipped.
    expect(screen.getAllByText("models.media_modality_disabled").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "models.kind_tts" }));
    expect(screen.getAllByText("models.media_modality_disabled").length).toBeGreaterThan(1);
  });

  it("warns that a [media] model scalar overrides the Model field", () => {
    setLoaded([]);
    setMediaEndpoints([
      { ...sampleMediaEndpoints[0], model_override: "whisper-1", model_override_path: "media.audio_model" },
    ]);
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_stt" }));
    expect(screen.getByText("models.media_model_overridden")).toBeInTheDocument();
  });

  it("locks the editor and hides Save in a managed deployment", () => {
    // #6695: every `POST /api/config/set` answers `423 config_managed`, so an
    // editable form here would only ever fail.
    useConfigStatusMock.mockReturnValue(makeQuery({ writable: false, source: "docker" }));
    setLoaded([]);
    setMediaEndpoints([sampleMediaEndpoints[0]]);
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_stt" }));

    expect(screen.getByText("config.managed_title")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "common.save" })).toBeNull();
    expect((screen.getByLabelText("models.media_base_url") as HTMLInputElement).readOnly).toBe(true);
  });

  it("marks the key_required toggle inert when no env var is named", () => {
    setLoaded([]);
    // sampleMediaEndpoints[1] (TTS) has an empty api_key_env.
    setMediaEndpoints([sampleMediaEndpoints[1]]);
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "models.kind_tts" }));
    expect(screen.getByText("models.media_key_required_inert")).toBeInTheDocument();

    const toggle = screen
      .getByText("models.media_key_required")
      .closest("label")!
      .querySelector("button") as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
  });

  it("surfaces a rejected media save as an error toast and keeps the drawer open", async () => {
    const save = makeMut({
      mutateAsync: vi.fn().mockRejectedValue(new Error("not user-tunable")),
    });
    useSaveMediaModelEndpointMock.mockReturnValue(save);
    setLoaded([]);
    setMediaEndpoints([sampleMediaEndpoints[0]]);
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "models.kind_stt" }));
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() =>
      expect(useUIStore.getState().toasts.map((toast) => toast.message)).toContain(
        "not user-tunable",
      ),
    );
    expect(screen.getByLabelText("models.media_base_url")).toBeInTheDocument();
  });
});

describe("ModelsPage helpers", () => {
  it("parses empty number inputs without coercing them to zero", () => {
    expect(numberInputValue("")).toBe("");
    expect(numberInputValue("0")).toBe(0);
    expect(numberInputValue("12.5")).toBe(12.5);
  });

  it("builds complete override state and compares it by value", () => {
    const first = settingsStateFromOverrides({ temperature: 0.4 });
    const equivalent = settingsStateFromOverrides({ temperature: 0.4 });
    const changed = settingsStateFromOverrides({ temperature: 0.8 });
    expect(first.tempEnabled).toBe(true);
    expect(settingsStateEqual(first, equivalent)).toBe(true);
    expect(settingsStateEqual(first, changed)).toBe(false);
  });

  it("formats capability override labels with explicit branches", () => {
    const labels = {
      auto: "Auto",
      on: "On",
      off: "Off",
      forceOn: "Force on",
      forceOff: "Force off",
    };
    expect(capabilityOverrideLabel("default", true, labels)).toBe("Auto (On)");
    expect(capabilityOverrideLabel("default", false, labels)).toBe("Auto (Off)");
    expect(capabilityOverrideLabel("on", false, labels)).toBe("Force on");
    expect(capabilityOverrideLabel("off", true, labels)).toBe("Force off");
  });
});
