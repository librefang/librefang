import React from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MediaPage, normalizeSpeechSpeed } from "./MediaPage";
import { useMediaProviders, useVideoTask } from "../lib/queries/media";
import {
  useGenerateImage,
  useGenerateMusic,
  useSubmitVideo,
  useSynthesizeSpeech,
} from "../lib/mutations/media";
import { useUIStore } from "../lib/store";

vi.mock("../lib/queries/media", () => ({
  useMediaProviders: vi.fn(),
  useVideoTask: vi.fn(),
}));

vi.mock("../lib/mutations/media", () => ({
  useGenerateImage: vi.fn(),
  useGenerateMusic: vi.fn(),
  useSubmitVideo: vi.fn(),
  useSynthesizeSpeech: vi.fn(),
}));

vi.mock("../lib/store", () => ({
  useUIStore: vi.fn(),
}));

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, options?: Record<string, unknown>) =>
        String(options?.defaultValue ?? key),
    }),
  };
});

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

const mock = <T,>(fn: T) => fn as unknown as ReturnType<typeof vi.fn>;
const useMediaProvidersMock = mock(useMediaProviders);
const useVideoTaskMock = mock(useVideoTask);
const useGenerateImageMock = mock(useGenerateImage);
const useGenerateMusicMock = mock(useGenerateMusic);
const useSubmitVideoMock = mock(useSubmitVideo);
const useSynthesizeSpeechMock = mock(useSynthesizeSpeech);
const useUIStoreMock = mock(useUIStore);

const imageMutate = vi.fn();
const speechMutate = vi.fn();
const addToast = vi.fn();

function mutation(mutate = vi.fn()) {
  return { mutate, isPending: false };
}

function finishImageGeneration(images: { data_base64: string; url?: string }[]) {
  const options = imageMutate.mock.calls[0][1];
  act(() => {
    options.onSuccess({
      images,
      model: "image-model",
      provider: "image-provider",
    });
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  useMediaProvidersMock.mockReturnValue({
    data: [
      {
        name: "configured-provider",
        configured: true,
        capabilities: [
          "image_generation",
          "text_to_speech",
          "video_generation",
          "music_generation",
        ],
      },
    ],
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  });
  useVideoTaskMock.mockReturnValue({
    data: undefined,
    error: null,
    isError: false,
  });
  useGenerateImageMock.mockReturnValue(mutation(imageMutate));
  useSynthesizeSpeechMock.mockReturnValue(mutation(speechMutate));
  useSubmitVideoMock.mockReturnValue(mutation());
  useGenerateMusicMock.mockReturnValue(mutation());
  useUIStoreMock.mockImplementation((selector: (state: { addToast: typeof addToast }) => unknown) =>
    selector({ addToast }),
  );
});

describe("MediaPage image results", () => {
  it("uses a sanitized URL for both the image and its link", () => {
    render(<MediaPage />);
    fireEvent.change(screen.getByPlaceholderText("media.image_prompt_placeholder"), {
      target: { value: "a sunrise" },
    });
    fireEvent.click(screen.getByRole("button", { name: "media.generate" }));
    finishImageGeneration([{ data_base64: "", url: "https://example.com/image.png" }]);

    const image = screen.getByRole("img", { name: "generated {{index}}" });
    expect(image).toHaveAttribute("src", "https://example.com/image.png");
    expect(image.closest("a")).toHaveAttribute("href", "https://example.com/image.png");
  });

  it("never assigns an unsafe result URL to an image or link", () => {
    render(<MediaPage />);
    fireEvent.change(screen.getByPlaceholderText("media.image_prompt_placeholder"), {
      target: { value: "a sunrise" },
    });
    fireEvent.click(screen.getByRole("button", { name: "media.generate" }));
    finishImageGeneration([{ data_base64: "", url: "javascript:alert(1)" }]);

    expect(screen.queryByRole("img", { name: "generated {{index}}" })).not.toBeInTheDocument();
    expect(document.querySelector('[href="javascript:alert(1)"]')).not.toBeInTheDocument();
    expect(screen.getByText("Image unavailable")).toBeInTheDocument();
  });

  it("renders an explicit unavailable state for an empty image payload", () => {
    render(<MediaPage />);
    fireEvent.change(screen.getByPlaceholderText("media.image_prompt_placeholder"), {
      target: { value: "a sunrise" },
    });
    fireEvent.click(screen.getByRole("button", { name: "media.generate" }));
    finishImageGeneration([{ data_base64: "" }]);

    expect(screen.getByText("Image unavailable")).toBeInTheDocument();
    expect(document.querySelector('img[src="data:image/png;base64,"]')).not.toBeInTheDocument();
  });

  it("does not construct a data URL for an oversized image payload", () => {
    render(<MediaPage />);
    fireEvent.change(screen.getByPlaceholderText("media.image_prompt_placeholder"), {
      target: { value: "a sunrise" },
    });
    fireEvent.click(screen.getByRole("button", { name: "media.generate" }));
    finishImageGeneration([{ data_base64: "a".repeat(2 * 1024 * 1024 + 1) }]);

    expect(screen.getByText("Image too large to display")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "generated {{index}}" })).not.toBeInTheDocument();
  });
});

describe("MediaPage speech speed", () => {
  it("normalizes finite values into the API range and omits non-finite values", () => {
    expect(normalizeSpeechSpeed(0)).toBe(0.25);
    expect(normalizeSpeechSpeed(2.5)).toBe(2.5);
    expect(normalizeSpeechSpeed(9)).toBe(4);
    expect(normalizeSpeechSpeed(Number.NaN)).toBeUndefined();
    expect(normalizeSpeechSpeed(Number.POSITIVE_INFINITY)).toBeUndefined();
  });

  it("submits the normalized speed", () => {
    const { container } = render(<MediaPage />);
    fireEvent.click(screen.getByRole("tab", { name: "media.tab_speech" }));
    fireEvent.change(screen.getByPlaceholderText("media.speech_text_placeholder"), {
      target: { value: "hello" },
    });
    const speedInput = container.querySelector('input[type="number"]');
    expect(speedInput).not.toBeNull();
    fireEvent.change(speedInput!, { target: { value: "10" } });
    fireEvent.submit(screen.getByRole("button", { name: "media.synthesize" }).closest("form")!);

    expect(speechMutate).toHaveBeenCalledWith(
      expect.objectContaining({ speed: 4 }),
      expect.any(Object),
    );
  });
});

describe("MediaPage video errors", () => {
  it("does not cross-suppress query and provider-status errors with the same text", () => {
    useVideoTaskMock.mockReturnValue({
      data: { status: "failed", error: "generation failed" },
      error: new Error("generation failed"),
      isError: true,
    });
    render(<MediaPage />);
    fireEvent.click(screen.getByRole("tab", { name: "media.tab_video" }));

    expect(addToast).toHaveBeenCalledTimes(2);
    expect(addToast).toHaveBeenNthCalledWith(1, "generation failed", "error");
    expect(addToast).toHaveBeenNthCalledWith(2, "generation failed", "error");
  });
});
