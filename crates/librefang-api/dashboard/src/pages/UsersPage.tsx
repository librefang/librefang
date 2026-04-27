// Users page (RBAC M6).
//
// Surfaces:
//   - List view with role filter + name/binding search
//   - Create / edit modal
//   - Delete confirmation
//   - Identity-linking wizard (4 steps)
//   - CSV bulk import (drag-drop preview + commit)
//   - Quick links to per-user budget / policy / simulator stubs
//
// All API access lives in `lib/queries/users.ts` and `lib/mutations/users.ts`.
// This file only renders.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import {
  Users,
  Plus,
  Search,
  X,
  UploadCloud,
  Wand2,
  KeyRound,
  Shield,
  RefreshCw,
  Copy,
  ListChecks,
  Database,
  Wallet,
  MoreVertical,
  ChevronDown,
  ChevronUp,
  Trash2,
} from "lucide-react";

import type { UserItem, UserUpsertPayload } from "../lib/http/client";
import { useUsers } from "../lib/queries/users";
import {
  useCreateUser,
  useDeleteUser,
  useImportUsers,
  useRotateUserKey,
  useUpdateUser,
} from "../lib/mutations/users";
import { parseUsersCsv } from "../lib/csvParser";
import { useUIStore } from "../lib/store";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";

// Single source of truth for the role enum the dashboard speaks to. Mirrors
// `librefang_kernel::auth::UserRole` and `UserConfig::role` (lower-case).
const ROLES = ["owner", "admin", "user", "viewer"] as const;
type RoleName = (typeof ROLES)[number];

// Each platform tile in the wizard advertises its expected platform_id
// shape so admins don't have to spelunk for the right format.
const PLATFORM_TILES: Array<{
  key: string;
  label: string;
  hint: string;
  example: string;
}> = [
  {
    key: "telegram",
    label: "Telegram",
    hint: "Numeric Telegram user ID (visible via @userinfobot).",
    example: "123456789",
  },
  {
    key: "discord",
    label: "Discord",
    hint: "Numeric Discord user ID (right-click → Copy User ID, dev mode).",
    example: "987654321098765432",
  },
  {
    key: "slack",
    label: "Slack",
    hint: "Slack member ID (Profile → More → Copy member ID).",
    example: "U01ABCDEFGH",
  },
  {
    key: "email",
    label: "Email",
    hint: "Sender email address (used by IMAP / Mailgun channels).",
    example: "alice@example.com",
  },
  {
    key: "wechat",
    label: "WeChat",
    hint: "WeCom / WeChat OpenID for the configured corp.",
    example: "abc123@im.wechat",
  },
];

export function UsersPage() {
  const { t } = useTranslation();
  const addToast = useUIStore(s => s.addToast);

  // ── state ────────────────────────────────────────────────────────────
  const [search, setSearch] = useState("");
  const [roleFilter, setRoleFilter] = useState<"all" | RoleName>("all");
  const [editing, setEditing] = useState<UserItem | null>(null);
  const [creating, setCreating] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<UserItem | null>(null);
  const [wizardUser, setWizardUser] = useState<UserItem | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  // Rotation flow has two distinct phases with distinct state. The
  // pre-confirm phase asks "are you sure?" via ConfirmDialog. The
  // post-confirm phase shows the plaintext key copy-once. Errors do NOT
  // hijack the success modal — they go through the toast system so the
  // operator's mental model stays clean: "modal open = you have a key".
  const [confirmRotate, setConfirmRotate] = useState<UserItem | null>(null);
  const [rotatedKey, setRotatedKey] =
    useState<{ name: string; plaintext: string } | null>(null);

  // ── data ─────────────────────────────────────────────────────────────
  const usersQuery = useUsers({
    role: roleFilter === "all" ? undefined : roleFilter,
    search,
  });

  const createMut = useCreateUser();
  const updateMut = useUpdateUser();
  const deleteMut = useDeleteUser();
  const rotateMut = useRotateUserKey();

  const users = usersQuery.data ?? [];

  const handleRefresh = useCallback(() => {
    void usersQuery.refetch();
  }, [usersQuery]);

  // ── render ──────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Users className="h-4 w-4" />}
        title={t("users.title", "Users & RBAC")}
        subtitle={t(
          "users.subtitle",
          "Manage operator accounts, channel bindings, and bulk-onboard via CSV.",
        )}
        badge={t("users.badge", "Phase 4 / M6")}
        isFetching={usersQuery.isFetching}
        onRefresh={handleRefresh}
        actions={
          <div className="flex flex-wrap gap-2">
            <Link
              to="/users/simulator"
              className="inline-flex items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 py-1.5 text-xs font-medium text-text-main hover:border-brand/30 hover:text-brand"
            >
              <Shield className="h-3.5 w-3.5" />
              {t("users.simulator_link", "Permission simulator")}
            </Link>
            <Button
              variant="secondary"
              size="sm"
              leftIcon={<UploadCloud className="h-3.5 w-3.5" />}
              onClick={() => setImportOpen(true)}
            >
              {t("users.import_csv", "Bulk import (CSV)")}
            </Button>
            <Button
              variant="primary"
              size="sm"
              leftIcon={<Plus className="h-3.5 w-3.5" />}
              onClick={() => setCreating(true)}
            >
              {t("users.create", "New user")}
            </Button>
          </div>
        }
        helpText={t(
          "users.help",
          "Each row maps a platform identity (Telegram / Discord / Slack / email) to a LibreFang role. Admin-only — endpoints live behind authenticated middleware.",
        )}
      />

      {/* Filter bar */}
      <Card padding="sm">
        <div className="flex flex-wrap gap-3 items-end">
          <div className="grow min-w-[220px]">
            <Input
              label={t("users.search_label", "Search")}
              placeholder={t(
                "users.search_placeholder",
                "Name or platform_id…",
              )}
              value={search}
              onChange={e => setSearch(e.target.value)}
              leftIcon={<Search className="h-3.5 w-3.5" />}
              rightIcon={
                search ? (
                  <button
                    type="button"
                    onClick={() => setSearch("")}
                    className="text-text-dim hover:text-text-main"
                    aria-label={t("common.clear", "Clear")}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                ) : null
              }
            />
          </div>
          <div className="w-40">
            <Select
              label={t("users.role_filter_label", "Role")}
              value={roleFilter}
              options={[
                { value: "all", label: t("users.all_roles", "All roles") },
                ...ROLES.map(r => ({
                  value: r,
                  label: t(`users.roles.${r}`, r),
                })),
              ]}
              onChange={e =>
                setRoleFilter(e.target.value as "all" | RoleName)
              }
            />
          </div>
        </div>
      </Card>

      {/* List */}
      {usersQuery.isPending ? (
        <div className="grid gap-4 md:grid-cols-2 stagger-children">
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : users.length === 0 ? (
        <EmptyState
          icon={<Users className="h-8 w-8" />}
          title={t("users.empty_title", "No users yet")}
          description={t(
            "users.empty_desc",
            "Add a user, then link a platform identity so chat events get attributed to a real role.",
          )}
        />
      ) : (
        <div className="grid gap-3 md:grid-cols-2 stagger-children">
          {users.map(u => (
            <Card key={u.name} hover padding="md">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <p className="text-sm font-bold truncate">{u.name}</p>
                    <Badge variant={roleVariant(u.role)}>
                      {t(`users.roles.${u.role}`, u.role)}
                    </Badge>
                    {u.has_api_key ? (
                      <Badge variant="brand">
                        <KeyRound className="h-3 w-3 mr-1 inline" />
                        {t("users.api_key", "API key")}
                      </Badge>
                    ) : null}
                    {u.has_policy ? (
                      <Badge
                        variant="info"
                        title={t(
                          "users.has_policy_title",
                          "User has a per-user tool policy / categories / channel rules override.",
                        )}
                      >
                        <ListChecks className="h-3 w-3 mr-1 inline" />
                        {t("users.has_policy_badge", "Policy")}
                      </Badge>
                    ) : null}
                    {u.has_memory_access ? (
                      <Badge
                        variant="info"
                        title={t(
                          "users.has_memory_title",
                          "User has a custom memory namespace ACL.",
                        )}
                      >
                        <Database className="h-3 w-3 mr-1 inline" />
                        {t("users.has_memory_badge", "Memory")}
                      </Badge>
                    ) : null}
                    {u.has_budget ? (
                      <Badge
                        variant="info"
                        title={t(
                          "users.has_budget_title",
                          "User has a per-user spend cap configured.",
                        )}
                      >
                        <Wallet className="h-3 w-3 mr-1 inline" />
                        {t("users.has_budget_badge", "Budget")}
                      </Badge>
                    ) : null}
                  </div>
                  <p className="mt-2 text-[11px] text-text-dim">
                    {Object.keys(u.channel_bindings).length} {t(
                      "users.bindings_suffix",
                      "channel binding(s)",
                    )}
                  </p>
                  {Object.entries(u.channel_bindings).length > 0 ? (
                    <ul className="mt-1 flex flex-wrap gap-1">
                      {Object.entries(u.channel_bindings).map(([k, v]) => (
                        <li
                          key={k}
                          className="font-mono text-[10px] rounded bg-main/40 px-1.5 py-0.5"
                          title={`${k}:${v}`}
                        >
                          {k}: {v}
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </div>
                <div className="flex flex-col gap-1.5 shrink-0 items-end">
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setEditing(u)}
                    >
                      {t("common.edit", "Edit")}
                    </Button>
                    <RowActionsMenu
                      user={u}
                      onLink={() => setWizardUser(u)}
                      onRotate={() => setConfirmRotate(u)}
                      onDelete={() => setConfirmDelete(u)}
                    />
                  </div>
                </div>
              </div>
              {/* Promoted "Budget" and "Permissions" affordances. Same
                  routes/queries as before — we just upgrade them from
                  11px footer text to ghost-button chips so their hit
                  area matches their importance. */}
              <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-border-subtle pt-2">
                <Link
                  to="/users/$name/budget"
                  params={{ name: u.name }}
                  className="inline-flex items-center gap-1 rounded-lg border border-border-subtle bg-surface px-2.5 py-1 text-[11px] font-medium text-text-dim hover:border-brand/40 hover:text-brand transition-colors"
                >
                  <Wallet className="h-3 w-3" />
                  {t("users.view_budget_chip", "Budget")}
                </Link>
                <Link
                  to="/users/$name/policy"
                  params={{ name: u.name }}
                  className="inline-flex items-center gap-1 rounded-lg border border-border-subtle bg-surface px-2.5 py-1 text-[11px] font-medium text-text-dim hover:border-brand/40 hover:text-brand transition-colors"
                >
                  <ListChecks className="h-3 w-3" />
                  {t("users.view_policy_chip", "Permissions")}
                </Link>
              </div>
            </Card>
          ))}
        </div>
      )}

      {/* Create / edit modal */}
      <UserFormModal
        isOpen={creating || editing !== null}
        editing={editing}
        onClose={() => {
          setCreating(false);
          setEditing(null);
        }}
        onSubmit={async payload => {
          if (editing) {
            await updateMut.mutateAsync({
              originalName: editing.name,
              payload,
            });
          } else {
            await createMut.mutateAsync(payload);
          }
          setCreating(false);
          setEditing(null);
        }}
        busy={createMut.isPending || updateMut.isPending}
      />

      {/* Identity wizard */}
      <IdentityWizardModal
        user={wizardUser}
        onClose={() => setWizardUser(null)}
        onCommit={async (user, channel, platformId) => {
          await updateMut.mutateAsync({
            originalName: user.name,
            payload: toUpsert(user, {
              channel_bindings: {
                ...user.channel_bindings,
                [channel]: platformId,
              },
            }),
          });
          setWizardUser(null);
        }}
        busy={updateMut.isPending}
      />

      {/* CSV import */}
      <BulkImportModal
        isOpen={importOpen}
        onClose={() => setImportOpen(false)}
      />

      {/* Delete confirm — uses shared ConfirmDialog for focus trap +
          keyboard semantics. Body composed as `message`. */}
      <ConfirmDialog
        isOpen={confirmDelete !== null}
        title={t("users.confirm_delete_title", "Delete user?")}
        message={
          confirmDelete
            ? `${confirmDelete.name} — ${t(
                "users.confirm_delete_body",
                "This removes the user from config.toml and rebuilds the RBAC channel index. Any platform identity that mapped to this user will fall through to the default-deny path.",
              )}`
            : ""
        }
        tone="destructive"
        confirmLabel={t("common.delete", "Delete")}
        onConfirm={async () => {
          if (!confirmDelete) return;
          await deleteMut.mutateAsync(confirmDelete.name);
        }}
        onClose={() => setConfirmDelete(null)}
      />

      {/* Rotate-key confirm — destructive tone (old key dies on
          confirm, can't be undone). On error: toast + close. On
          success: open the copy-once modal below. */}
      <ConfirmDialog
        isOpen={confirmRotate !== null}
        title={t("users.confirm_rotate_title", "Rotate API key?")}
        message={
          confirmRotate
            ? `${confirmRotate.name} — ${t(
                "users.confirm_rotate_body",
                "Generates a fresh API key for this user. The old key stops working immediately — any client still using it will start receiving 401 errors on the next request. The new plaintext key will be shown ONCE on the next screen; the server cannot reproduce it later.",
              )}`
            : ""
        }
        tone="destructive"
        confirmLabel={t("users.rotate_key_confirm", "Rotate now")}
        onConfirm={async () => {
          const target = confirmRotate;
          if (!target) return;
          try {
            const res = await rotateMut.mutateAsync(target.name);
            setRotatedKey({
              name: target.name,
              plaintext: res.new_api_key,
            });
          } catch (e) {
            // Surface as a toast — does NOT contaminate the success
            // modal which has hardened copy-once semantics. Common cause:
            // caller is Admin, not Owner.
            const msg = e instanceof Error ? e.message : String(e);
            addToast(
              t("users.rotate_key_failed_toast", "Rotation failed: {{message}}", {
                message: msg,
              }),
              "error",
            );
          }
        }}
        onClose={() => setConfirmRotate(null)}
      />

      {/* Post-rotation: copy-once display of the new plaintext key.
          Hardened: no backdrop dismiss, Esc swallowed, Close hidden
          until the operator has copied at least once. */}
      <RotatedKeyModal
        rotatedKey={rotatedKey}
        onClose={() => setRotatedKey(null)}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Post-rotate copy-once modal
// ---------------------------------------------------------------------------
//
// The plaintext key is shown exactly once — the daemon can't reproduce it.
// We treat it as load-bearing UI: any accidental dismissal would silently
// leave the operator with a working API key they can't read.
//
// Hardening:
//   - `disableBackdropClose` — backdrop click is a no-op
//   - `hideCloseButton` while `hasCopied` is false (no X in header)
//   - Esc keydown intercepted in the capture phase and `preventDefault`-ed
//   - Primary "I've copied the key" button is the only dismissal path
//     until Copy has been clicked

function RotatedKeyModal({
  rotatedKey,
  onClose,
}: {
  rotatedKey: { name: string; plaintext: string } | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore(s => s.addToast);
  const [hasCopied, setHasCopied] = useState(false);

  // Reset the gate every time a new key shows up. Using a ref-keyed
  // useEffect keeps this in sync with the prop without leaking state
  // across rotations.
  const lastName = useRef<string | null>(null);
  useEffect(() => {
    if (rotatedKey && lastName.current !== rotatedKey.name) {
      lastName.current = rotatedKey.name;
      setHasCopied(false);
    } else if (!rotatedKey) {
      lastName.current = null;
    }
  }, [rotatedKey]);

  // Capture-phase Escape interceptor. The shared Modal listens for Esc
  // on `window` and unconditionally closes; we attach in the capture
  // phase on `document` so we run first and `stopImmediatePropagation`
  // before the Modal's bubble-phase handler ever fires. Only active
  // while the modal is up AND the user hasn't copied yet.
  useEffect(() => {
    if (!rotatedKey || hasCopied) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
      }
    };
    document.addEventListener("keydown", handler, true);
    return () => document.removeEventListener("keydown", handler, true);
  }, [rotatedKey, hasCopied]);

  const handleCopy = useCallback(() => {
    if (!rotatedKey) return;
    void navigator.clipboard.writeText(rotatedKey.plaintext);
    setHasCopied(true);
    addToast(t("common.copied", "Copied"), "success");
  }, [rotatedKey, addToast, t]);

  return (
    <Modal
      isOpen={rotatedKey !== null}
      onClose={onClose}
      title={t("users.rotate_key_done_title", "New API key — copy now")}
      size="md"
      disableBackdropClose
      hideCloseButton={!hasCopied}
    >
      {rotatedKey ? (
        <div className="space-y-4 p-5">
          <div className="rounded-xl border border-warning/40 bg-warning/10 p-3">
            <p className="text-sm text-warning font-bold">
              {t(
                "users.rotate_key_only_chance",
                "This is your only chance to copy this key. Closing this dialog will discard the plaintext — the server cannot reproduce it.",
              )}
            </p>
          </div>
          <p className="text-xs text-text-dim">
            {t("users.rotate_key_done_user", "User")}:{" "}
            <span className="font-mono">{rotatedKey.name}</span>
          </p>
          <div className="flex items-center gap-2">
            <code className="grow rounded bg-main/40 px-3 py-2 font-mono text-xs break-all">
              {rotatedKey.plaintext}
            </code>
            <Button
              variant="secondary"
              size="sm"
              leftIcon={<Copy className="h-3.5 w-3.5" />}
              onClick={handleCopy}
            >
              {hasCopied ? t("common.copied", "Copied") : t("common.copy", "Copy")}
            </Button>
          </div>
          {!hasCopied ? (
            <p className="text-[11px] text-text-dim">
              {t(
                "users.rotate_key_must_copy",
                "Click Copy first. The Close button unlocks once the key is on your clipboard.",
              )}
            </p>
          ) : null}
          <div className="flex justify-end">
            <Button
              variant="primary"
              disabled={!hasCopied}
              title={
                !hasCopied
                  ? t(
                      "users.rotate_key_must_copy",
                      "Click Copy first. The Close button unlocks once the key is on your clipboard.",
                    )
                  : undefined
              }
              onClick={onClose}
            >
              {hasCopied
                ? t("users.rotate_key_copied_confirm", "Copied — you can close now")
                : t("users.rotate_key_done_close", "I've copied the key")}
            </Button>
          </div>
        </div>
      ) : null}
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function roleVariant(
  role: string,
): "brand" | "success" | "warning" | "error" | "info" {
  switch (role.toLowerCase()) {
    case "owner":
      return "error";
    case "admin":
      return "warning";
    case "viewer":
      return "info";
    default:
      return "success";
  }
}

// ---------------------------------------------------------------------------
// Row-level overflow menu
// ---------------------------------------------------------------------------
//
// Edit gets pulled out of this menu and stays as a primary affordance on the
// card. Everything else (Link, Rotate, Delete) lives behind a kebab to keep
// the visual weight of each row tight. We use `<details>` rather than a
// custom dropdown primitive because:
//   - it gives us click-to-toggle + click-outside-to-close + Esc-close for
//     free, all keyboard-accessible without an extra library
//   - no shared dropdown component exists in `components/ui/`, and the brief
//     forbids modifying primitives
//   - one menu per card on a list page — we don't need the bells of cmdk
//
// Trade-off: <details> doesn't have a built-in arrow-key navigation between
// items, but operators on this surface tab through and Enter/Space, which
// `<button>` children handle natively.

function RowActionsMenu({
  user,
  onLink,
  onRotate,
  onDelete,
}: {
  user: UserItem;
  onLink: () => void;
  onRotate: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const detailsRef = useRef<HTMLDetailsElement>(null);

  // Close the menu on outside click / Escape. <details> keeps `open`
  // toggled, so we just flip the attribute.
  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (
        detailsRef.current &&
        !detailsRef.current.contains(e.target as Node)
      ) {
        detailsRef.current.removeAttribute("open");
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && detailsRef.current?.hasAttribute("open")) {
        detailsRef.current.removeAttribute("open");
      }
    };
    document.addEventListener("click", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const close = () => detailsRef.current?.removeAttribute("open");

  return (
    <details ref={detailsRef} className="relative">
      <summary
        className="list-none cursor-pointer h-7 w-7 inline-flex items-center justify-center rounded-lg text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
        aria-label={t("users.row_actions", "More actions")}
        title={t("users.row_actions", "More actions")}
      >
        <MoreVertical className="h-3.5 w-3.5" />
      </summary>
      <div
        role="menu"
        className="absolute right-0 z-20 mt-1 w-48 overflow-hidden rounded-xl border border-border-subtle bg-surface shadow-lg shadow-black/10"
      >
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            close();
            onLink();
          }}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-text-main hover:bg-surface-hover"
        >
          <Wand2 className="h-3.5 w-3.5" />
          {t("users.link", "Link identity")}
        </button>
        {user.has_api_key ? (
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              close();
              onRotate();
            }}
            className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-text-main hover:bg-surface-hover"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {t("users.rotate_key", "Rotate API key")}
          </button>
        ) : null}
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            close();
            onDelete();
          }}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-error hover:bg-error/10"
        >
          <Trash2 className="h-3.5 w-3.5" />
          {t("common.delete", "Delete")}
        </button>
      </div>
    </details>
  );
}

function toUpsert(
  base: UserItem,
  overrides: Partial<UserUpsertPayload> = {},
): UserUpsertPayload {
  return {
    name: base.name,
    role: base.role,
    channel_bindings: { ...base.channel_bindings },
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Create / edit modal
// ---------------------------------------------------------------------------

function UserFormModal({
  isOpen,
  editing,
  onClose,
  onSubmit,
  busy,
}: {
  isOpen: boolean;
  editing: UserItem | null;
  onClose: () => void;
  onSubmit: (payload: UserUpsertPayload) => Promise<void>;
  busy: boolean;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [role, setRole] = useState<RoleName>("user");
  const [bindings, setBindings] = useState<Array<[string, string]>>([]);
  const [error, setError] = useState<string | null>(null);

  // Reset form when modal toggles or `editing` changes.
  const lastInit = useRef<{ key: string; editing: UserItem | null }>({
    key: "",
    editing: null,
  });
  if (isOpen) {
    const key = `${editing?.name ?? "__new__"}|${editing?.role ?? ""}`;
    if (lastInit.current.key !== key) {
      lastInit.current = { key, editing };
      setName(editing?.name ?? "");
      setRole(((editing?.role as RoleName) ?? "user") as RoleName);
      setBindings(
        editing
          ? Object.entries(editing.channel_bindings)
          : [],
      );
      setError(null);
    }
  }

  const submit = async () => {
    setError(null);
    if (!name.trim()) {
      setError(t("users.err_name_required", "Name is required."));
      return;
    }
    try {
      const channel_bindings: Record<string, string> = {};
      for (const [k, v] of bindings) {
        if (k.trim() && v.trim()) channel_bindings[k.trim()] = v.trim();
      }
      await onSubmit({
        name: name.trim(),
        role,
        channel_bindings,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={
        editing
          ? t("users.edit_title", "Edit user")
          : t("users.create_title", "New user")
      }
      size="lg"
      variant="panel-right"
    >
      <div className="space-y-4">
        <Input
          label={t("users.name_label", "Name")}
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder="alice"
          disabled={busy}
        />
        <Select
          label={t("users.role_label", "Role")}
          value={role}
          options={ROLES.map(r => ({
            value: r,
            label: t(`users.roles.${r}`, r),
          }))}
          onChange={e => setRole(e.target.value as RoleName)}
          disabled={busy}
        />
        <div>
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("users.bindings_label", "Channel bindings")}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setBindings([...bindings, ["telegram", ""]])}
              disabled={busy}
            >
              {t("common.add", "Add")}
            </Button>
          </div>
          {bindings.length === 0 ? (
            <p className="text-[11px] text-text-dim">
              {t(
                "users.bindings_empty",
                "No bindings. Use the identity wizard for guided platform_id formats.",
              )}
            </p>
          ) : (
            <ul className="space-y-2">
              {bindings.map(([k, v], i) => (
                <li key={i} className="flex gap-2 items-center">
                  <Select
                    value={k}
                    options={PLATFORM_TILES.map(p => ({
                      value: p.key,
                      label: t(`users.platforms.${p.key}.label`, p.label),
                    }))}
                    onChange={e => {
                      const next = [...bindings];
                      next[i] = [e.target.value, v];
                      setBindings(next);
                    }}
                    disabled={busy}
                    className="w-32"
                  />
                  <input
                    className="grow rounded-xl border border-border-subtle bg-surface px-3 py-2 text-sm font-mono"
                    value={v}
                    placeholder="platform_id"
                    onChange={e => {
                      const next = [...bindings];
                      next[i] = [k, e.target.value];
                      setBindings(next);
                    }}
                    disabled={busy}
                  />
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const next = [...bindings];
                      next.splice(i, 1);
                      setBindings(next);
                    }}
                    disabled={busy}
                    aria-label={t("common.remove", "Remove")}
                  >
                    <X className="h-3 w-3" />
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
        {error ? (
          <p className="text-xs text-error">{error}</p>
        ) : null}
        <div className="flex gap-2 justify-end pt-2 border-t border-border-subtle">
          <Button variant="secondary" onClick={onClose} disabled={busy}>
            {t("common.cancel", "Cancel")}
          </Button>
          <Button variant="primary" isLoading={busy} onClick={submit}>
            {editing
              ? t("common.save", "Save")
              : t("common.create", "Create")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Identity-linking wizard
// ---------------------------------------------------------------------------

function IdentityWizardModal({
  user,
  onClose,
  onCommit,
  busy,
}: {
  user: UserItem | null;
  onClose: () => void;
  onCommit: (
    user: UserItem,
    channel: string,
    platformId: string,
  ) => Promise<void>;
  busy: boolean;
}) {
  const { t } = useTranslation();
  // Two-step wizard: 0 = Platform, 1 = Identifier (input + ack + Save).
  // The old 4-step flow (User → Platform → Identifier → Confirm) had a
  // wasted "User" step (no input) and an "Identifier" / "Confirm" split
  // that just retyped the value. The user is now a fixed header chip
  // outside the step body so the operator never loses context.
  const [step, setStep] = useState(0);
  const [channel, setChannel] = useState<string>("telegram");
  const [platformId, setPlatformId] = useState("");
  const [error, setError] = useState<string | null>(null);
  // "Why?" disclosure for the ownership warning. Hidden by default to
  // de-emphasize the wall of text on first view; the ack checkbox + a
  // short headline still make the risk plain.
  const [showWarning, setShowWarning] = useState(false);
  // Operator must explicitly attest they've checked the platform_id belongs
  // to the target user. There's no automated ownership check (no bot DM
  // challenge yet), so an Owner could otherwise socially-engineer attribution
  // by binding a stranger's telegram_id to a user row. See PR #3209 follow-up.
  const [acknowledged, setAcknowledged] = useState(false);

  // Reset when target user changes.
  const lastUser = useRef<string | null>(null);
  if (user && lastUser.current !== user.name) {
    lastUser.current = user.name;
    setStep(0);
    setChannel("telegram");
    setPlatformId("");
    setError(null);
    setShowWarning(false);
    setAcknowledged(false);
  } else if (!user) {
    lastUser.current = null;
  }

  const tile = PLATFORM_TILES.find(p => p.key === channel);
  const stepLabels = [
    t("users.wizard_step_platform", "Platform"),
    t("users.wizard_step_identity", "Identifier"),
  ];

  return (
    <Modal
      isOpen={user !== null}
      onClose={onClose}
      title={t("users.wizard_title", "Link a platform identity")}
      size="lg"
      variant="panel-right"
    >
      {user ? (
        <div className="space-y-4">
          {/* User chip — fixed header so the operator never loses
              context about which user they're binding to. Replaces
              the old "User" wizard step. */}
          <Card padding="md">
            <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("users.wizard_user_chip", "Linking identity to")}
            </p>
            <div className="mt-1 flex items-center gap-2 flex-wrap">
              <span className="text-sm font-bold">{user.name}</span>
              <Badge variant={roleVariant(user.role)}>
                {t(`users.roles.${user.role}`, user.role)}
              </Badge>
              <span className="text-[11px] text-text-dim">
                {Object.keys(user.channel_bindings).length}{" "}
                {t("users.bindings_suffix", "channel binding(s)")}
              </span>
            </div>
          </Card>

          <ol className="flex items-center gap-2 text-[10px] uppercase tracking-widest text-text-dim">
            {stepLabels.map((label, i) => {
              const isCurrent = i === step;
              const isPast = i < step;
              const clickable = isPast; // forward-gating preserved
              return (
                <li
                  key={label}
                  className={`flex items-center gap-1 ${
                    isCurrent ? "text-brand font-bold" : ""
                  }`}
                >
                  {clickable ? (
                    <button
                      type="button"
                      onClick={() => setStep(i)}
                      className="flex items-center gap-1 hover:text-brand"
                      aria-label={t("users.wizard_step_back_aria", "Go back to step {{n}}", { n: i + 1 })}
                    >
                      <span className="w-5 h-5 rounded-full text-[10px] flex items-center justify-center bg-brand/20 text-brand">
                        {i + 1}
                      </span>
                      {label}
                    </button>
                  ) : (
                    <>
                      <span
                        className={`w-5 h-5 rounded-full text-[10px] flex items-center justify-center ${
                          isCurrent
                            ? "bg-brand/20 text-brand"
                            : "bg-main/30 text-text-dim"
                        }`}
                      >
                        {i + 1}
                      </span>
                      {label}
                    </>
                  )}
                  {i < stepLabels.length - 1 ? (
                    <span className="opacity-30">›</span>
                  ) : null}
                </li>
              );
            })}
          </ol>

          {step === 0 ? (
            <div className="space-y-2">
              <p className="text-xs text-text-dim">
                {t(
                  "users.wizard_platform_desc",
                  "Pick which platform's identifier you're linking.",
                )}
              </p>
              <div className="grid grid-cols-2 gap-2">
                {PLATFORM_TILES.map(p => (
                  <button
                    key={p.key}
                    type="button"
                    onClick={() => setChannel(p.key)}
                    className={`text-left p-3 rounded-xl border transition-colors ${
                      channel === p.key
                        ? "border-brand bg-brand/10"
                        : "border-border-subtle hover:border-brand/30"
                    }`}
                  >
                    <p className="text-sm font-bold">
                      {t(`users.platforms.${p.key}.label`, p.label)}
                    </p>
                    <p className="mt-1 text-[11px] text-text-dim">
                      {t(`users.platforms.${p.key}.hint`, p.hint)}
                    </p>
                  </button>
                ))}
              </div>
            </div>
          ) : null}

          {step === 1 ? (
            <div className="space-y-3">
              {/* Format hint card — kept from old step 2. */}
              {tile ? (
                <Card padding="md">
                  <p className="text-sm font-bold">
                    {t(`users.platforms.${tile.key}.label`, tile.label)}
                  </p>
                  <p className="mt-1 text-[11px] text-text-dim">
                    {t(`users.platforms.${tile.key}.hint`, tile.hint)}
                  </p>
                  <p className="mt-2 text-[11px] font-mono text-text-dim">
                    {t("users.wizard_example", "Example")}:{" "}
                    {t(`users.platforms.${tile.key}.example`, tile.example)}
                  </p>
                </Card>
              ) : null}

              <Input
                label={t("users.wizard_id_label", "platform_id")}
                value={platformId}
                onChange={e => setPlatformId(e.target.value)}
                placeholder={
                  tile
                    ? t(`users.platforms.${tile.key}.example`, tile.example)
                    : undefined
                }
              />

              {/* Ack checkbox always visible — the warning detail is
                  gated behind a "Why?" disclosure to keep the form
                  scannable. The headline still names the risk. */}
              <div className="rounded-xl border border-warning/40 bg-warning/10 p-3 text-xs space-y-2">
                <div className="flex items-start justify-between gap-2">
                  <p className="font-bold text-warning">
                    {t(
                      "users.wizard_unverified_title",
                      "No automated ownership check",
                    )}
                  </p>
                  <button
                    type="button"
                    onClick={() => setShowWarning(s => !s)}
                    className="inline-flex items-center gap-1 text-[11px] text-warning hover:underline shrink-0"
                  >
                    {showWarning
                      ? t("users.wizard_hide_warning", "Hide warning")
                      : t("users.wizard_show_warning", "Why this matters")}
                    {showWarning ? (
                      <ChevronUp className="h-3 w-3" />
                    ) : (
                      <ChevronDown className="h-3 w-3" />
                    )}
                  </button>
                </div>
                {showWarning ? (
                  <p className="text-text-dim">
                    {t(
                      "users.wizard_unverified_body",
                      "LibreFang does not yet ping the platform to confirm this id belongs to {{user}}. Anyone with Owner rights can bind any id to any user row, which silently retargets future RBAC and rate-limit decisions. Verify the platform_id with the user out-of-band before saving.",
                      { user: user.name },
                    )}
                  </p>
                ) : null}
                <label className="flex items-start gap-2 cursor-pointer pt-1">
                  <input
                    type="checkbox"
                    className="mt-0.5"
                    checked={acknowledged}
                    onChange={e => setAcknowledged(e.target.checked)}
                    disabled={busy}
                  />
                  <span className="text-text">
                    {t(
                      "users.wizard_unverified_ack",
                      "I have verified out-of-band that {{platformId}} belongs to {{user}}.",
                      {
                        platformId: platformId || "this id",
                        user: user.name,
                      },
                    )}
                  </span>
                </label>
              </div>
            </div>
          ) : null}

          {error ? <p className="text-xs text-error">{error}</p> : null}

          <div className="flex gap-2 justify-between pt-2 border-t border-border-subtle">
            <Button
              variant="ghost"
              onClick={() => setStep(s => Math.max(0, s - 1))}
              disabled={step === 0 || busy}
            >
              {t("common.back", "Back")}
            </Button>
            {step < 1 ? (
              <Button
                variant="primary"
                onClick={() => {
                  setError(null);
                  setStep(1);
                }}
              >
                {t("common.next", "Next")}
              </Button>
            ) : (
              <Button
                variant="primary"
                isLoading={busy}
                disabled={!acknowledged || !platformId.trim()}
                title={
                  !platformId.trim()
                    ? t("users.err_id_required", "platform_id is required.")
                    : !acknowledged
                      ? t(
                          "users.wizard_ack_required",
                          "Acknowledge the ownership warning to save.",
                        )
                      : undefined
                }
                onClick={async () => {
                  if (!platformId.trim()) {
                    setError(
                      t("users.err_id_required", "platform_id is required."),
                    );
                    return;
                  }
                  if (!acknowledged) {
                    setError(
                      t(
                        "users.wizard_ack_required",
                        "Acknowledge the ownership warning to save.",
                      ),
                    );
                    return;
                  }
                  setError(null);
                  try {
                    await onCommit(user, channel, platformId.trim());
                  } catch (e) {
                    setError(e instanceof Error ? e.message : String(e));
                  }
                }}
              >
                {t("users.wizard_commit", "Save binding")}
              </Button>
            )}
          </div>
        </div>
      ) : null}
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Bulk-import (CSV) modal
// ---------------------------------------------------------------------------

function BulkImportModal({
  isOpen,
  onClose,
}: {
  isOpen: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [rawText, setRawText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const importMut = useImportUsers();

  const parsed = useMemo(() => parseUsersCsv(rawText, ROLES), [rawText]);

  const onFile = (file: File) => {
    const reader = new FileReader();
    reader.onload = () => {
      setRawText(typeof reader.result === "string" ? reader.result : "");
    };
    reader.onerror = () => {
      setError(
        t(
          "users.csv_read_failed",
          "Could not read file — try pasting the contents instead.",
        ),
      );
    };
    reader.readAsText(file);
  };

  const result = importMut.data;

  return (
    <Modal
      isOpen={isOpen}
      onClose={() => {
        importMut.reset();
        setRawText("");
        setError(null);
        onClose();
      }}
      title={t("users.import_title", "Bulk import users")}
      size="3xl"
      variant="panel-right"
    >
      <div className="space-y-4">
        <p className="text-xs text-text-dim">
          {t(
            "users.import_desc",
            "Drop a CSV with columns name,role,telegram,discord,slack,email. Roles must be one of owner / admin / user / viewer.",
          )}
        </p>

        <DropZone onFile={onFile} />

        <div>
          <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
            {t("users.csv_paste", "Or paste CSV")}
          </label>
          <textarea
            className="mt-1.5 w-full font-mono text-xs rounded-xl border border-border-subtle bg-surface p-3 min-h-[160px]"
            value={rawText}
            onChange={e => setRawText(e.target.value)}
            placeholder={t(
              "users.csv_paste_placeholder",
              "alice,user,telegram=123456789;email=alice@example.com",
            )}
          />
        </div>

        {error ? <p className="text-xs text-error">{error}</p> : null}

        {parsed.rows.length > 0 ? (
          <Card padding="md">
            <p className="text-[10px] font-black uppercase tracking-widest text-text-dim mb-2">
              {t("users.import_preview", "Preview")}
            </p>
            <ul className="space-y-1 text-xs max-h-48 overflow-auto">
              {parsed.rows.map((r, i) => (
                <li key={i} className="flex gap-2">
                  <span className="font-mono text-text-dim w-6">{i + 1}</span>
                  <span className="font-bold">{r.name}</span>
                  <Badge variant={roleVariant(r.role)}>
                    {t(`users.roles.${r.role}`, r.role)}
                  </Badge>
                  <span className="text-text-dim">
                    {t("users.import_bindings_count", "{{count}} bindings", {
                      count: Object.keys(r.channel_bindings ?? {}).length,
                    })}
                  </span>
                </li>
              ))}
            </ul>
            {parsed.errors.length > 0 ? (
              <ul className="mt-2 space-y-0.5 text-[11px] text-error">
                {parsed.errors.map((m, i) => (
                  <li key={i}>• {m}</li>
                ))}
              </ul>
            ) : null}
          </Card>
        ) : null}

        {result ? (
          <Card padding="md">
            <p className="text-sm font-bold">
              {result.dry_run
                ? t("users.import_dry_summary", "Dry-run summary")
                : t("users.import_summary", "Import complete")}
            </p>
            <p className="mt-1 text-xs text-text-dim">
              {t(
                "users.import_result_counts",
                "{{created}} created · {{updated}} updated · {{failed}} failed",
                {
                  created: result.created,
                  updated: result.updated,
                  failed: result.failed,
                },
              )}
            </p>
            {result.rows.some(r => r.error) ? (
              <ul className="mt-2 space-y-0.5 text-[11px] text-error">
                {result.rows
                  .filter(r => r.error)
                  .map(r => (
                    <li key={r.index}>
                      {t(
                        "users.import_row_error",
                        "row {{row}} ({{name}}): {{error}}",
                        {
                          row: r.index + 1,
                          name: r.name,
                          error: r.error,
                        },
                      )}
                    </li>
                  ))}
              </ul>
            ) : null}
          </Card>
        ) : null}

        <div className="flex gap-2 justify-end pt-2 border-t border-border-subtle">
          <Button variant="secondary" onClick={onClose}>
            {t("common.close", "Close")}
          </Button>
          <Button
            variant="ghost"
            isLoading={importMut.isPending}
            disabled={parsed.rows.length === 0}
            onClick={() =>
              importMut.mutate({ rows: parsed.rows, dryRun: true })
            }
          >
            {t("users.import_dry_run", "Dry run")}
          </Button>
          <Button
            variant="primary"
            isLoading={importMut.isPending}
            disabled={parsed.rows.length === 0}
            onClick={() =>
              importMut.mutate({ rows: parsed.rows, dryRun: false })
            }
          >
            {t("users.import_commit", "Commit")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function DropZone({ onFile }: { onFile: (file: File) => void }) {
  const { t } = useTranslation();
  const [active, setActive] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  // Native <button type="button"> gives us Enter / Space → click for free,
  // plus a real focus ring without `tabIndex` plumbing. Drag handlers stay
  // on the button itself; the underlying file `<input>` is hidden but
  // remains the actual file picker that the click forwards to.
  return (
    <>
      <button
        type="button"
        onDragOver={e => {
          e.preventDefault();
          setActive(true);
        }}
        onDragLeave={() => setActive(false)}
        onDrop={e => {
          e.preventDefault();
          setActive(false);
          const f = e.dataTransfer.files?.[0];
          if (f) onFile(f);
        }}
        onClick={() => inputRef.current?.click()}
        aria-label={t(
          "users.csv_drop_aria",
          "Upload CSV file: drop here or activate to browse",
        )}
        className={`w-full block cursor-pointer rounded-xl border-2 border-dashed p-6 text-center text-xs transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-brand focus-visible:ring-offset-2 focus-visible:ring-offset-surface ${
          active
            ? "border-brand bg-brand/10 text-brand"
            : "border-border-subtle text-text-dim hover:border-brand/30"
        }`}
      >
        <UploadCloud className="mx-auto mb-2 h-6 w-6" />
        <p>
          {t(
            "users.csv_drop",
            "Drop a CSV here, or click to browse.",
          )}
        </p>
      </button>
      <input
        ref={inputRef}
        type="file"
        accept=".csv,text/csv"
        className="hidden"
        onChange={e => {
          const f = e.target.files?.[0];
          if (f) onFile(f);
          e.target.value = "";
        }}
      />
    </>
  );
}

// Tiny CSV parser tuned for the import shape: header row + simple rows. We
// don't pull in a full CSV library because the dashboard ships zero-bundle
// hot paths and the import body shape is a documented narrow contract.
// CSV parsing now lives in `lib/csvParser.ts` so it can be unit-tested for
// quoted-newline + BOM handling without dragging in React.
