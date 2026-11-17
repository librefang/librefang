import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ReasoningEffortMenu } from "./ReasoningEffortMenu";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { defaultValue?: string } | Record<string, unknown>) => {
        if (opts && typeof opts === "object" && "defaultValue" in opts) {
          return (opts as { defaultValue?: string }).defaultValue ?? key;
        }
        return key;
      },
      i18n: { language: "en" },
    }),
  };
});

function openMenu(props: Partial<Parameters<typeof ReasoningEffortMenu>[0]> = {}) {
  const onUseOnce = vi.fn();
  const onPin = vi.fn();
  const onUnpin = vi.fn();
  render(
    <ReasoningEffortMenu
      onUseOnce={onUseOnce}
      onPin={onPin}
      onUnpin={onUnpin}
      {...props}
    />,
  );
  return { onUseOnce, onPin, onUnpin };
}

async function openPopover(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByTestId("reasoning-effort-button"));
}

describe("ReasoningEffortMenu — level list", () => {
  it("offers the canonical levels when the model declares none", async () => {
    const user = userEvent.setup();
    openMenu();
    await openPopover(user);

    // The fallback vocabulary the agreed interface names.
    for (const name of ["None", "Minimal", "Low", "Medium", "High", "xHigh", "Max"]) {
      expect(screen.getByRole("menuitemradio", { name })).toBeInTheDocument();
    }
    expect(screen.getAllByRole("menuitemradio")).toHaveLength(8); // + Default
  });

  it("offers the active model's declared levels instead", async () => {
    const user = userEvent.setup();
    openMenu({ efforts: ["minimal", "medium", "xhigh", "max"] });
    await openPopover(user);

    for (const name of ["Minimal", "Medium", "xHigh", "Max"]) {
      expect(screen.getByRole("menuitemradio", { name })).toBeInTheDocument();
    }
    // A level the model does not accept is not offered.
    expect(screen.queryByRole("menuitemradio", { name: "Low" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitemradio", { name: "High" })).not.toBeInTheDocument();
  });

  // A stored level the catalog does not list still has a row — the operator
  // must be able to see what is set — but it is not *offered*: the model does
  // not declare it, so it is marked, disabled, and cannot be re-armed or
  // pinned from here.
  it("marks a stored level the catalog does not list without offering it", async () => {
    const user = userEvent.setup();
    const { onUseOnce, onPin } = openMenu({ sessionReasoningEffort: "ultra" });
    await openPopover(user);

    const row = screen.getByRole("menuitemradio", { name: "Ultra" });
    expect(row).toHaveAttribute("aria-checked", "true");
    expect(row).toBeDisabled();
    await user.click(row);
    expect(onUseOnce).not.toHaveBeenCalled();

    expect(screen.getByTestId("reasoning-pin-ultra")).toBeDisabled();
    await user.click(screen.getByTestId("reasoning-pin-ultra"));
    expect(onPin).not.toHaveBeenCalled();
  });
});

describe("ReasoningEffortMenu — marks", () => {
  it("marks the session-pinned level as the effective one", async () => {
    const user = userEvent.setup();
    openMenu({ sessionReasoningEffort: "max" });
    await openPopover(user);

    expect(screen.getByRole("menuitemradio", { name: "Max" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByTestId("reasoning-pin-max")).toHaveAttribute("aria-pressed", "true");
    // The pill carries the pinned level and the pin itself.
    const pill = screen.getByTestId("reasoning-effort-button");
    expect(within(pill).getByText("Max")).toBeInTheDocument();
    expect(screen.getByTestId("reasoning-pin-indicator")).toBeInTheDocument();
  });

  it("lets the one-shot level outrank the pin, and says so on the pill", async () => {
    const user = userEvent.setup();
    openMenu({ nextReasoningEffort: "high", sessionReasoningEffort: "max" });
    await openPopover(user);

    expect(screen.getByRole("menuitemradio", { name: "High" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("menuitemradio", { name: "Max" })).toHaveAttribute(
      "aria-checked",
      "false",
    );

    await user.keyboard("{Escape}");
    const pill = screen.getByTestId("reasoning-effort-button");
    expect(within(pill).getByText("High")).toBeInTheDocument();
    expect(screen.getByTestId("reasoning-next-badge")).toBeInTheDocument();
    expect(screen.getByTestId("reasoning-pin-indicator")).toBeInTheDocument();
  });

  it("marks Default only when no level is in effect at all", async () => {
    const user = userEvent.setup();
    openMenu();
    await openPopover(user);

    expect(screen.getByRole("menuitemradio", { name: "chat.reasoning_default" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });
});

describe("ReasoningEffortMenu — actions", () => {
  it("arms a level for the next message only, and closes", async () => {
    const user = userEvent.setup();
    const { onUseOnce, onPin } = openMenu({ sessionReasoningEffort: "max" });
    await openPopover(user);

    await user.click(screen.getByRole("menuitemradio", { name: "High" }));

    expect(onUseOnce).toHaveBeenCalledWith("high");
    // Arming is not pinning: the session state was not touched.
    expect(onPin).not.toHaveBeenCalled();
    expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();
  });

  it("clears the one-shot with Default", async () => {
    const user = userEvent.setup();
    const { onUseOnce } = openMenu({ nextReasoningEffort: "high" });
    await openPopover(user);

    await user.click(screen.getByRole("menuitemradio", { name: "chat.reasoning_default" }));

    expect(onUseOnce).toHaveBeenCalledWith(null);
  });

  it("pins the level clicked on its pin affordance", async () => {
    const user = userEvent.setup();
    const { onPin, onUseOnce } = openMenu();
    await openPopover(user);

    await user.click(screen.getByTestId("reasoning-pin-xhigh"));

    expect(onPin).toHaveBeenCalledWith("xhigh");
    // Pinning is not a one-shot: no next-message arming happened.
    expect(onUseOnce).not.toHaveBeenCalled();
  });

  it("shows the pinned banner and unpins from it", async () => {
    const user = userEvent.setup();
    const { onUnpin } = openMenu({ sessionReasoningEffort: "max" });
    await openPopover(user);

    expect(screen.getByText("chat.reasoning_pinned")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "chat.reasoning_unpin" }));

    expect(onUnpin).toHaveBeenCalledTimes(1);
  });

  it("holds the pin actions while the session patch is in flight", async () => {
    const user = userEvent.setup();
    const { onPin } = openMenu({ pinPending: true });
    await openPopover(user);

    const pin = screen.getByTestId("reasoning-pin-high");
    expect(pin).toBeDisabled();
    await user.click(pin);
    expect(onPin).not.toHaveBeenCalled();
  });
});

describe("ReasoningEffortMenu — degraded backends", () => {
  it("renders without any session field and without pin wiring", async () => {
    const user = userEvent.setup();
    render(<ReasoningEffortMenu />);
    await user.click(screen.getByTestId("reasoning-effort-button"));

    // The menu still lists the canonical levels…
    expect(screen.getByRole("menuitemradio", { name: "None" })).toBeInTheDocument();
    // …but offers no pin affordance that would PATCH nothing.
    expect(screen.queryByTestId("reasoning-pin-none")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "chat.reasoning_unpin" })).not.toBeInTheDocument();
  });

  it("surfaces a failed session patch without closing the menu", async () => {
    const user = userEvent.setup();
    openMenu({ pinError: "chat.reasoning_pin_failed" });
    await openPopover(user);

    expect(screen.getByText("chat.reasoning_pin_failed")).toBeInTheDocument();
  });
});
