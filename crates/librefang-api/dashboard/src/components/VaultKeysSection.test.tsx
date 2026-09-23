import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { VaultKeysSection } from "./VaultKeysSection";
import { useVaultKeys } from "../lib/queries/vault";
import { useDeleteVaultKey, useSetVaultKey } from "../lib/mutations/vault";
import { ApiError } from "../lib/http/errors";

// i18next is not initialised under vitest, so its `t` would hand back the
// default string with the `{{key}}` placeholders intact. Substitute them the
// way the real runtime does, otherwise the accessible names asserted below
// would be testing the mock rather than the component.
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, defaultValue: string, opts?: Record<string, unknown>) =>
      Object.entries(opts ?? {}).reduce(
        (acc, [name, value]) => acc.split(`{{${name}}}`).join(String(value)),
        defaultValue,
      ),
  }),
}));

vi.mock("../lib/queries/vault", () => ({ useVaultKeys: vi.fn() }));
vi.mock("../lib/mutations/vault", () => ({
  useSetVaultKey: vi.fn(),
  useDeleteVaultKey: vi.fn(),
}));

const mockUseVaultKeys = vi.mocked(useVaultKeys);
const mockUseSetVaultKey = vi.mocked(useSetVaultKey);
const mockUseDeleteVaultKey = vi.mocked(useDeleteVaultKey);

const SECRET = "ghp_notARealTokenJustATestFixture";

type QueryStub = { data?: unknown; isError?: boolean; error?: unknown; isLoading?: boolean };

function stubQuery(over: QueryStub = {}) {
  mockUseVaultKeys.mockReturnValue({
    data: [{ key: "GITHUB_TOKEN", set: false, source: "unset" }],
    isError: false,
    error: null,
    isLoading: false,
    ...over,
  } as ReturnType<typeof useVaultKeys>);
}

function stubMutations(
  setImpl = vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: true, source: "vault" }),
  deleteImpl = vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: false, removed: true, source: "unset" }),
) {
  mockUseSetVaultKey.mockReturnValue({
    mutateAsync: setImpl,
    isPending: false,
  } as unknown as ReturnType<typeof useSetVaultKey>);
  mockUseDeleteVaultKey.mockReturnValue({
    mutateAsync: deleteImpl,
    isPending: false,
  } as unknown as ReturnType<typeof useDeleteVaultKey>);
  return setImpl;
}

describe("VaultKeysSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    stubQuery();
    stubMutations();
  });

  it("renders the keys the API reported rather than a hard-coded list", () => {
    stubQuery({
      data: [
        { key: "GITHUB_TOKEN", set: true, source: "vault" },
        { key: "SOME_FUTURE_KEY", set: false, source: "unset" },
      ],
    });
    render(<VaultKeysSection />);
    expect(screen.getByText("GITHUB_TOKEN")).toBeInTheDocument();
    // A key the client has never heard of appears purely because the daemon
    // listed it — this is what makes extending WRITABLE_KEYS a one-side change.
    expect(screen.getByText("SOME_FUTURE_KEY")).toBeInTheDocument();
  });

  it("shows set / not set without ever rendering a value", () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "vault" }] });
    const { container } = render(<VaultKeysSection />);
    expect(screen.getByText("Set")).toBeInTheDocument();
    const input = screen.getByLabelText("New value for GITHUB_TOKEN") as HTMLInputElement;
    // Never prefilled: there is no read-back endpoint, and a mask at the real
    // length would leak the length.
    expect(input.value).toBe("");
    expect(input.type).toBe("password");
    expect(container.textContent).not.toMatch(/[•*]{3,}/);
  });

  it("clears the input on save so the secret is not left in the DOM", async () => {
    const setImpl = stubMutations();
    const { container } = render(<VaultKeysSection />);
    const input = screen.getByLabelText("New value for GITHUB_TOKEN") as HTMLInputElement;

    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(setImpl).toHaveBeenCalledWith({ key: "GITHUB_TOKEN", value: SECRET }));
    await waitFor(() => expect(input.value).toBe(""));
    expect(container.innerHTML).not.toContain(SECRET);
    expect(container.textContent).not.toContain(SECRET);
  });

  it("keeps the secret out of the error message when the write fails", async () => {
    stubMutations(vi.fn().mockRejectedValue(new ApiError(503, "vault", "Vault unavailable: locked")));
    render(<VaultKeysSection />);
    const input = screen.getByLabelText("New value for GITHUB_TOKEN") as HTMLInputElement;

    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    const message = await screen.findByText("Vault unavailable: locked");
    expect(message.textContent).not.toContain(SECRET);
  });

  it("keeps the secret out of the generic fallback message too", async () => {
    // An ApiError is an Error, so the assertion above only ever exercises the
    // `e.message` branch. Reject with a non-Error to reach the fallback string,
    // which is the one a careless edit would interpolate the value into.
    stubMutations(vi.fn().mockRejectedValue("boom"));
    render(<VaultKeysSection />);
    const input = screen.getByLabelText("New value for GITHUB_TOKEN") as HTMLInputElement;

    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    // Scoped to the message: a failed write deliberately leaves the draft in
    // the input so the operator can retry, so the container still holds it.
    const message = await screen.findByText("Could not store the secret.");
    expect(message.textContent).not.toContain(SECRET);
    expect(message.innerHTML).not.toContain(SECRET);
  });

  it("explains the Owner gate instead of rendering a dead form on 403", () => {
    stubQuery({ data: undefined, isError: true, error: new ApiError(403, "forbidden", "nope") });
    render(<VaultKeysSection />);
    expect(
      screen.getByText("Managing daemon credentials requires an Owner account."),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/New value for/)).not.toBeInTheDocument();
  });

  it("requires a second click before removing a stored secret", async () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "vault" }] });
    render(<VaultKeysSection />);
    const del = mockUseDeleteVaultKey.mock.results[0].value.mutateAsync;

    fireEvent.click(screen.getByRole("button", { name: "Remove GITHUB_TOKEN" }));
    expect(del).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(del).toHaveBeenCalledWith({ key: "GITHUB_TOKEN" }));
  });

  // Nothing but a successful delete or an explicit Cancel used to clear
  // `confirmDelete`, so a confirmation the operator opened and then abandoned
  // survived a save on the same row and stayed one click from firing — against
  // the secret that had just been stored.
  it("disarms an abandoned delete confirmation when the row is saved instead", async () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "vault" }] });
    render(<VaultKeysSection />);
    const del = mockUseDeleteVaultKey.mock.results[0].value.mutateAsync;

    // Arm the delete, then change your mind and store a new value instead.
    fireEvent.click(screen.getByRole("button", { name: "Remove GITHUB_TOKEN" }));
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/New value for/), {
      target: { value: SECRET },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Confirm" })).not.toBeInTheDocument(),
    );
    expect(del).not.toHaveBeenCalled();
  });

  // ── Effective source (#8186 review) ──────────────────────────────────────
  //
  // The daemon reads its own environment before the vault, so a badge built
  // from `set` reported "Not set" on a deployment where promotion worked, and
  // reported it again after a delete that revoked nothing.

  it("says the environment overrides the key instead of reporting it unset", () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: false, source: "environment" }] });
    render(<VaultKeysSection />);
    expect(screen.getByText("Overridden by the environment")).toBeInTheDocument();
    expect(screen.queryByText("Not set")).not.toBeInTheDocument();
    expect(screen.getByText(/reads GITHUB_TOKEN from its own environment/)).toBeInTheDocument();
  });

  it("says a stored copy is inert when the environment wins", () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "environment" }] });
    render(<VaultKeysSection />);
    expect(screen.getByText("Overridden by the environment")).toBeInTheDocument();
    expect(screen.queryByText("Set")).not.toBeInTheDocument();
    expect(screen.getByText(/A vault copy is stored but inert\./)).toBeInTheDocument();
  });

  it("leaves a vault-sourced key badged as set with no override notice", () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "vault" }] });
    render(<VaultKeysSection />);
    expect(screen.getByText("Set")).toBeInTheDocument();
    expect(screen.queryByText(/own environment/)).not.toBeInTheDocument();
  });

  it("does not confirm a write as effective when the environment still overrides", async () => {
    stubMutations(vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: true, source: "environment" }));
    render(<VaultKeysSection />);
    const input = screen.getByLabelText("New value for GITHUB_TOKEN") as HTMLInputElement;

    fireEvent.change(input, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    const message = await screen.findByText(
      /GITHUB_TOKEN stored, but the daemon reads it from its own environment/,
    );
    expect(message.textContent).not.toContain(SECRET);
  });

  it("does not claim a delete revoked anything the environment still supplies", async () => {
    stubMutations(
      undefined,
      vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: false, removed: true, source: "environment" }),
    );
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true, source: "environment" }] });
    render(<VaultKeysSection />);

    fireEvent.click(screen.getByRole("button", { name: "Remove GITHUB_TOKEN" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));

    expect(
      await screen.findByText(/removed from the vault, but the daemon still reads it/),
    ).toBeInTheDocument();
  });
});
