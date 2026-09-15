import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React, { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "./ConfirmDialog";
import { Modal } from "./Modal";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get:
        (_target, tag: string) =>
        ({
          children,
          initial: _initial,
          animate: _animate,
          exit: _exit,
          transition: _transition,
          variants: _variants,
          ...rest
        }: Record<string, unknown> & { children?: React.ReactNode }) =>
          React.createElement(tag, rest, children),
    },
  ),
}));

/// A `ConfirmDialog` stacked on a `Modal`, the arrangement 19 dashboard files
/// build and the one #8336 is about.
function ModalWithNestedConfirm({ onConfirm }: { onConfirm?: () => void }) {
  const [modalOpen, setModalOpen] = useState(true);
  const [confirmOpen, setConfirmOpen] = useState(true);
  return (
    <>
      {modalOpen && <div data-testid="modal-open-marker" />}
      <Modal isOpen={modalOpen} onClose={() => setModalOpen(false)} title="History">
        <p>version list</p>
      </Modal>
      <ConfirmDialog
        isOpen={confirmOpen}
        title="Restore this version?"
        message="The current manifest will be replaced."
        onConfirm={() => onConfirm?.()}
        onClose={() => setConfirmOpen(false)}
      />
    </>
  );
}

describe("Escape across stacked dismissable layers (#8336)", () => {
  it("closes only the dialog, leaving the modal underneath open", async () => {
    const user = userEvent.setup();
    render(<ModalWithNestedConfirm />);

    expect(screen.getByText("Restore this version?")).toBeTruthy();
    expect(screen.getByTestId("modal-open-marker")).toBeTruthy();

    await user.keyboard("{Escape}");

    // The dialog is gone...
    expect(screen.queryByText("Restore this version?")).toBeNull();
    // ...and the modal it was opened from is not. Before the shared stack,
    // `Modal`'s handler ran first, called `stopImmediatePropagation`, and closed
    // the layer *underneath* — taking the dialog with it and dropping the user
    // out of the context they were working in.
    expect(screen.queryByTestId("modal-open-marker")).not.toBeNull();
  });

  it("closes the modal on a second Escape, once the dialog is gone", async () => {
    const user = userEvent.setup();
    render(<ModalWithNestedConfirm />);

    await user.keyboard("{Escape}");
    await user.keyboard("{Escape}");

    expect(screen.queryByText("Restore this version?")).toBeNull();
    expect(screen.queryByTestId("modal-open-marker")).toBeNull();
  });

  it("does not confirm the dialog with Enter while another layer is stacked above it", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();

    function ConfirmUnderModal() {
      return (
        <>
          <ConfirmDialog
            isOpen
            title="Promote this agent type?"
            message="It will be published to the registry."
            onConfirm={onConfirm}
            onClose={() => {}}
          />
          {/* Above the dialog: z-index 200 beats the dialog's 150. */}
          <Modal isOpen onClose={() => {}} title="Details" zIndex={200}>
            <p>on top</p>
          </Modal>
        </>
      );
    }

    render(<ConfirmUnderModal />);
    await user.keyboard("{Enter}");

    // Enter belongs to whatever is on top, the same as Escape. Reaching through
    // to confirm a dialog the user cannot even see is the same defect one key
    // over — and on a non-destructive dialog it would have fired the mutation.
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("still confirms with Enter when the dialog is the topmost layer", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();

    render(
      <ConfirmDialog
        isOpen
        title="Promote this agent type?"
        message="It will be published to the registry."
        onConfirm={onConfirm}
        onClose={() => {}}
      />,
    );
    await user.keyboard("{Enter}");

    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
