import { StrictMode } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AuxiliaryLlmSection } from "./AuxiliaryLlmSection";

const { configData, mutateAsync, addToast } = vi.hoisted(() => ({
  configData: {
    llm: {
      auxiliary: {
        compression: ["  groq:big  ", "", "openai:gpt-small"],
      },
    },
  },
  mutateAsync: vi.fn(),
  addToast: vi.fn(),
}));

vi.mock("../lib/queries/config", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../lib/queries/config")>();
  return {
    ...actual,
    useAuxiliaryChains: () => ({
      data: actual.selectAuxiliaryChains(configData),
    }),
    useConfigSchema: () => ({
      data: { "x-aux-tasks": ["compression", "title"] },
    }),
  };
});

vi.mock("../lib/queries/models", () => ({
  useModels: () => ({
    data: {
      models: [
        { id: "gpt-small", provider: "openai", display_name: "GPT Small" },
        { id: "big", provider: "groq", display_name: undefined },
      ],
    },
  }),
}));

vi.mock("../lib/queries/providers", () => ({
  useProviders: () => ({
    data: [{ id: "groq" }, { id: "openai" }],
  }),
}));

vi.mock("../lib/mutations/config", () => ({
  useSetConfigValue: () => ({
    mutateAsync,
    isPending: false,
  }),
}));

vi.mock("../lib/store", () => ({
  useUIStore: (sel: (s: unknown) => unknown) => sel({ addToast }),
}));

vi.mock("../lib/errors", () => ({
  toastErr: (_err: unknown, fallback: string) => fallback,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback: string) =>
      typeof fallback === "string" ? fallback : _key,
  }),
}));

function renderSection() {
  return render(
    <StrictMode>
      <AuxiliaryLlmSection />
    </StrictMode>,
  );
}

describe("AuxiliaryLlmSection", () => {
  beforeEach(() => {
    mutateAsync.mockResolvedValue({ status: "applied_partial" });
    addToast.mockClear();
    mutateAsync.mockClear();
  });

  it("renders one row per kernel-served aux task", () => {
    renderSection();
    expect(screen.getByText("compression")).toBeInTheDocument();
    expect(screen.getByText("title")).toBeInTheDocument();
    // A task the kernel does not serve must not appear.
    expect(screen.queryByText("session_summary")).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Edit" })).toHaveLength(2);
  });

  it("shows the primary-model fallback for an unconfigured task", () => {
    renderSection();
    const row = screen.getByText("title").closest("div");
    expect(within(row as HTMLElement).getByText("Primary (default)")).toBeInTheDocument();
  });

  it("saves with path llm.auxiliary.<task> and a filtered chain", async () => {
    renderSection();
    const row = screen.getByText("compression").closest("div");
    fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "Edit" }));

    const inputs = screen.getAllByPlaceholderText("provider:model");
    expect(inputs).toHaveLength(4); // 3 stored entries + 1 empty draft row
    expect(inputs[0]).toHaveValue("  groq:big  ");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await vi.waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        path: "llm.auxiliary.compression",
        value: ["groq:big", "openai:gpt-small"],
      });
    });
  });

  it("rejects a save whose provider prefix is not registered", async () => {
    renderSection();
    const row = screen.getByText("compression").closest("div");
    fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "Edit" }));

    const inputs = screen.getAllByPlaceholderText("provider:model");
    expect(inputs).toHaveLength(4); // 3 stored + 1 empty draft row
    fireEvent.change(inputs[3], { target: { value: "bogus:model" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await vi.waitFor(() => {
      expect(addToast).toHaveBeenCalledWith(
        expect.stringContaining("Unknown provider"),
        "error",
      );
    });
    expect(mutateAsync).not.toHaveBeenCalled();
  });

  it("suggests provider:model entries from the model catalog", () => {
    renderSection();
    const row = screen.getByText("compression").closest("div");
    fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "Edit" }));

    const options = Array.from(document.querySelectorAll("datalist option"));
    const values = options.map((o) => o.getAttribute("value"));
    expect(values).toContain("openai:gpt-small");
    expect(values).toContain("groq:big");
  });

  it("clears the editing row when Save succeeds", async () => {
    renderSection();
    const row = screen.getByText("compression").closest("div");
    fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await vi.waitFor(() => {
      expect(addToast).toHaveBeenCalledWith("Saved", "success");
    });
    expect(screen.queryByPlaceholderText("provider:model")).not.toBeInTheDocument();
  });
});