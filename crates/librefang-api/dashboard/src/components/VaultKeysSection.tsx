import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle, KeyRound, Save, Trash2, XCircle } from "lucide-react";

import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { ApiError } from "../lib/http/client";
import { useVaultKeys } from "../lib/queries/vault";
import { useDeleteVaultKey, useSetVaultKey } from "../lib/mutations/vault";

/**
 * Operator control for the daemon's credential vault (#8164).
 *
 * The value of a stored secret is never rendered, not even masked at its real
 * length — the daemon has no read-back endpoint, and a mask sized to the real
 * value would leak the length. The only state shown is the key name and a
 * set/not-set boolean, exactly what `GET /api/vault/keys` returns.
 *
 * The draft input is cleared on a successful write rather than repopulated,
 * because there is nothing to repopulate it from and leaving the typed secret
 * sitting in a live DOM node after it has been stored serves no purpose.
 *
 * The key list comes from the response, never from a client-side constant, so
 * extending the daemon's `WRITABLE_KEYS` allowlist surfaces the new key here
 * with no dashboard change.
 */
export function VaultKeysSection() {
  const { t } = useTranslation();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const vaultQuery = useVaultKeys();
  const setKey = useSetVaultKey();
  const deleteKey = useDeleteVaultKey();

  const keys = vaultQuery.data ?? [];
  const busy = setKey.isPending || deleteKey.isPending;

  // The endpoints are Admin-gated in-handler, so a viewer-role operator gets a
  // 401/403 rather than an empty list. Say so instead of rendering a dead form.
  const forbidden =
    vaultQuery.isError &&
    vaultQuery.error instanceof ApiError &&
    (vaultQuery.error.status === 401 || vaultQuery.error.status === 403);

  function draftOf(key: string) {
    return drafts[key] ?? "";
  }

  async function handleSave(key: string) {
    const value = draftOf(key).trim();
    if (busy || !value) return;
    setError(null);
    setSuccess(null);
    try {
      await setKey.mutateAsync({ key, value });
      // Clear before anything else can read it back out of state.
      setDrafts((prev) => ({ ...prev, [key]: "" }));
      setSuccess(t("settings.vault_saved", "{{key}} stored.", { key }));
    } catch (e) {
      // Only the server's message, never the value the operator typed.
      setError(
        e instanceof Error
          ? e.message
          : t("settings.vault_save_failed", "Could not store the secret."),
      );
    }
  }

  async function handleDelete(key: string) {
    if (busy) return;
    setError(null);
    setSuccess(null);
    try {
      await deleteKey.mutateAsync({ key });
      setDrafts((prev) => ({ ...prev, [key]: "" }));
      setConfirmDelete(null);
      setSuccess(t("settings.vault_removed", "{{key}} removed.", { key }));
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message
          : t("settings.vault_delete_failed", "Could not remove the secret."),
      );
    }
  }

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface">
      <div className="px-5 py-3 border-b border-border-subtle/50">
        <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
          {t("settings.vault", "Credential Vault")}
        </p>
      </div>
      <div className="px-5">
        <div className="flex items-start gap-4 py-4 border-b border-border-subtle/50">
          <KeyRound className="w-4 h-4 shrink-0 text-amber-500" />
          <div className="flex-1 min-w-0">
            <p className="text-sm font-semibold">
              {t("settings.vault_title", "Daemon credentials")}
            </p>
            <p className="text-xs text-text-dim mt-0.5">
              {t(
                "settings.vault_desc",
                "Secrets the daemon needs for its own outbound work, such as the GitHub token that skill proposal and agent-type promotion require. Stored encrypted; never shown again after saving.",
              )}
            </p>
          </div>
        </div>

        {forbidden && (
          <div className="px-1 py-3 text-sm text-text-dim">
            {t(
              "settings.vault_forbidden",
              "Managing daemon credentials requires an Admin account.",
            )}
          </div>
        )}

        {!forbidden && vaultQuery.isError && (
          <div className="px-1 py-3 text-sm text-danger">
            {t("settings.vault_load_failed", "Could not load vault keys.")}
          </div>
        )}

        {!forbidden && !vaultQuery.isError && keys.length === 0 && (
          <div className="px-1 py-3 text-sm text-text-dim">
            {vaultQuery.isLoading
              ? t("common.loading", "Loading...")
              : t(
                  "settings.vault_empty",
                  "This daemon exposes no writable vault keys.",
                )}
          </div>
        )}

        {!forbidden &&
          keys.map((entry) => (
            <div key={entry.key} className="py-4 space-y-2">
              <div className="flex items-center gap-3">
                <p className="text-sm font-semibold font-mono truncate">
                  {entry.key}
                </p>
                <Badge variant={entry.set ? "success" : "default"}>
                  {entry.set ? (
                    <CheckCircle className="w-3 h-3 mr-1" />
                  ) : (
                    <XCircle className="w-3 h-3 mr-1" />
                  )}
                  {entry.set
                    ? t("settings.vault_is_set", "Set")
                    : t("settings.vault_not_set", "Not set")}
                </Badge>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  value={draftOf(entry.key)}
                  onChange={(e) =>
                    setDrafts((prev) => ({
                      ...prev,
                      [entry.key]: e.target.value,
                    }))
                  }
                  aria-label={t("settings.vault_input_label", "New value for {{key}}", {
                    key: entry.key,
                  })}
                  placeholder={
                    entry.set
                      ? t(
                          "settings.vault_placeholder_replace",
                          "Enter a new value to replace the stored one",
                        )
                      : t("settings.vault_placeholder_new", "Paste the secret")
                  }
                  className="flex-1 rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-mono focus:outline-none focus:ring-2 focus:ring-brand/30"
                />
                <Button
                  variant="primary"
                  size="sm"
                  className="shrink-0"
                  disabled={busy || !draftOf(entry.key).trim()}
                  onClick={() => handleSave(entry.key)}
                >
                  <Save className="w-4 h-4 mr-1" />
                  {t("settings.vault_save", "Save")}
                </Button>
                {entry.set &&
                  (confirmDelete === entry.key ? (
                    <>
                      <Button
                        variant="danger"
                        size="sm"
                        className="shrink-0"
                        disabled={busy}
                        onClick={() => handleDelete(entry.key)}
                      >
                        {t("settings.vault_confirm_remove", "Confirm")}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="shrink-0"
                        disabled={busy}
                        onClick={() => setConfirmDelete(null)}
                      >
                        {t("common.cancel", "Cancel")}
                      </Button>
                    </>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="shrink-0"
                      aria-label={t("settings.vault_remove", "Remove {{key}}", {
                        key: entry.key,
                      })}
                      disabled={busy}
                      onClick={() => setConfirmDelete(entry.key)}
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  ))}
              </div>
            </div>
          ))}

        {error && <div className="px-1 pb-3 text-sm text-danger">{error}</div>}
        {success && (
          <div className="px-1 pb-3 text-sm text-emerald-500">{success}</div>
        )}
      </div>
    </div>
  );
}
