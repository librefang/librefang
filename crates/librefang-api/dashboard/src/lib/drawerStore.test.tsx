import { afterEach, describe, expect, it, vi } from "vitest";

import { useDrawerStore } from "./drawerStore";

afterEach(() => {
  useDrawerStore.setState({ isOpen: false, content: null });
});

describe("drawer store replacement", () => {
  it("notifies the previous owner when another drawer replaces it", async () => {
    const onClose = vi.fn();
    const firstOwner = {};

    useDrawerStore.getState().open({ body: "first", owner: firstOwner, onClose });
    useDrawerStore.getState().open({ body: "second", owner: {} });

    expect(onClose).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not notify when the same owner refreshes its content", async () => {
    const onClose = vi.fn();
    const owner = {};

    useDrawerStore.getState().open({ body: "first", owner, onClose });
    useDrawerStore.getState().open({ body: "updated", owner, onClose });

    await Promise.resolve();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("suppresses replacement notification when the old owner closes itself", async () => {
    const onClose = vi.fn();
    const firstOwner = {};

    useDrawerStore.getState().open({ body: "first", owner: firstOwner, onClose });
    useDrawerStore.getState().open({ body: "second", owner: {} });
    useDrawerStore.getState().close(firstOwner);

    await Promise.resolve();
    expect(onClose).not.toHaveBeenCalled();
    expect(useDrawerStore.getState().content?.body).toBe("second");
  });

  it("clears retained content when the active drawer closes", () => {
    useDrawerStore.getState().open({ body: "first" });

    useDrawerStore.getState().close();

    expect(useDrawerStore.getState()).toMatchObject({ isOpen: false, content: null });
  });
});
