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
    data: [{ key: "GITHUB_TOKEN", set: false }],
    isError: false,
    error: null,
    isLoading: false,
    ...over,
  } as ReturnType<typeof useVaultKeys>);
}

function stubMutations(setImpl = vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: true })) {
  mockUseSetVaultKey.mockReturnValue({
    mutateAsync: setImpl,
    isPending: false,
  } as unknown as ReturnType<typeof useSetVaultKey>);
  mockUseDeleteVaultKey.mockReturnValue({
    mutateAsync: vi.fn().mockResolvedValue({ key: "GITHUB_TOKEN", set: false, removed: true }),
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
        { key: "GITHUB_TOKEN", set: true },
        { key: "SOME_FUTURE_KEY", set: false },
      ],
    });
    render(<VaultKeysSection />);
    expect(screen.getByText("GITHUB_TOKEN")).toBeInTheDocument();
    // A key the client has never heard of appears purely because the daemon
    // listed it — this is what makes extending WRITABLE_KEYS a one-side change.
    expect(screen.getByText("SOME_FUTURE_KEY")).toBeInTheDocument();
  });

  it("shows set / not set without ever rendering a value", () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true }] });
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

  it("explains the Admin gate instead of rendering a dead form on 403", () => {
    stubQuery({ data: undefined, isError: true, error: new ApiError(403, "forbidden", "nope") });
    render(<VaultKeysSection />);
    expect(
      screen.getByText("Managing daemon credentials requires an Admin account."),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/New value for/)).not.toBeInTheDocument();
  });

  it("requires a second click before removing a stored secret", async () => {
    stubQuery({ data: [{ key: "GITHUB_TOKEN", set: true }] });
    render(<VaultKeysSection />);
    const del = mockUseDeleteVaultKey.mock.results[0].value.mutateAsync;

    fireEvent.click(screen.getByRole("button", { name: "Remove GITHUB_TOKEN" }));
    expect(del).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(del).toHaveBeenCalledWith({ key: "GITHUB_TOKEN" }));
  });
});
