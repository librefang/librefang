import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Typewriter_v2 } from "./Typewriter_v2";

const { animateMock, mathPluginsMock } = vi.hoisted(() => ({
  animateMock: vi.fn(),
  mathPluginsMock: vi.fn(() => ({ remarkPlugins: [], rehypePlugins: [] })),
}));

vi.mock("motion/react", () => ({ animate: animateMock }));
vi.mock("../lib/hooks/useMathPlugins", () => ({
  useMathPlugins: mathPluginsMock,
}));

interface AnimationOptions {
  duration: number;
  onUpdate: (latest: number) => void;
  onComplete: () => void;
}

function latestAnimation(): AnimationOptions {
  return animateMock.mock.calls[animateMock.mock.calls.length - 1]?.[2] as AnimationOptions;
}

describe("Typewriter_v2", () => {
  beforeEach(() => {
    animateMock.mockReset();
    animateMock.mockReturnValue({ stop: vi.fn() });
    mathPluginsMock.mockClear();
  });

  it("batches markdown renders and always commits the final text", () => {
    render(<Typewriter_v2 text="abcdefghij" speed={10} />);
    const animation = latestAnimation();

    act(() => animation.onUpdate(4));
    expect(screen.queryByText("abcd")).toBeNull();

    act(() => animation.onUpdate(5));
    expect(screen.getByText("abcde")).toBeInTheDocument();

    act(() => animation.onUpdate(9));
    expect(screen.queryByText("abcdefghi")).toBeNull();

    act(() => animation.onComplete());
    expect(screen.getByText("abcdefghij")).toBeInTheDocument();
  });

  it("selects math plugins from the rendered substring", () => {
    render(<Typewriter_v2 text="$x$ suffix" speed={10} />);
    expect(mathPluginsMock).toHaveBeenLastCalledWith("");

    act(() => latestAnimation().onUpdate(5));
    expect(mathPluginsMock).toHaveBeenLastCalledWith("$x$ s");
  });

  it("rewinds without a synchronous flush when the source shrinks", () => {
    const view = render(<Typewriter_v2 text="abcdefghij" speed={10} />);
    act(() => latestAnimation().onUpdate(10));

    view.rerender(<Typewriter_v2 text="xy" speed={10} />);
    expect(screen.queryByText("abcdefghij")).toBeNull();
    expect(animateMock).toHaveBeenLastCalledWith(0, 2, expect.any(Object));
  });

  it("applies a changed speed to the next text update", () => {
    const view = render(<Typewriter_v2 text="abcdefghij" speed={10} />);
    expect(latestAnimation().duration).toBe(0.1);

    view.rerender(<Typewriter_v2 text="abcdefghij" speed={20} />);
    expect(animateMock).toHaveBeenCalledTimes(1);

    view.rerender(<Typewriter_v2 text="abcdefghijk" speed={20} />);
    expect(latestAnimation().duration).toBe(0.22);
  });
});
