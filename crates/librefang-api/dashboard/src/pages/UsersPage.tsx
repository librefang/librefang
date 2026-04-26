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

import { useCallback, useMemo, useRef, useState } from "react";
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
  ListChecks,
  Database,
  Wallet,
} from "lucide-react";

import type { UserItem, UserUpsertPayload } from "../lib/http/client";
import { useUsers } from "../lib/queries/users";
import {
  useCreateUser,
  useDeleteUser,
  useImportUsers,
  useUpdateUser,
} from "../lib/mutations/users";
import { parseUsersCsv } from "../lib/csvParser";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { Modal } from "../components/ui/Modal";
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

  // ── state ────────────────────────────────────────────────────────────
  const [search, setSearch] = useState("");
  const [roleFilter, setRoleFilter] = useState<"all" | RoleName>("all");
  const [editing, setEditing] = useState<UserItem | null>(null);
  const [creating, setCreating] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<UserItem | null>(null);
  const [wizardUser, setWizardUser] = useState<UserItem | null>(null);
  const [importOpen, setImportOpen] = useState(false);

  // ── data ─────────────────────────────────────────────────────────────
  const usersQuery = useUsers({
    role: roleFilter === "all" ? undefined : roleFilter,
    search,
  });

  const createMut = useCreateUser();
  const updateMut = useUpdateUser();
  const deleteMut = useDeleteUser();

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
                ...ROLES.map(r => ({ value: r, label: r })),
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
                    <Badge variant={roleVariant(u.role)}>{u.role}</Badge>
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
                  <Button
                    variant="ghost"
                    size="sm"
                    leftIcon={<Wand2 className="h-3 w-3" />}
                    onClick={() => setWizardUser(u)}
                  >
                    {t("users.link", "Link identity")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setEditing(u)}
                  >
                    {t("common.edit", "Edit")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setConfirmDelete(u)}
                  >
                    {t("common.delete", "Delete")}
                  </Button>
                </div>
              </div>
              <div className="mt-3 flex flex-wrap gap-2 border-t border-border-subtle pt-2">
                <Link
                  to="/users/$name/budget"
                  params={{ name: u.name }}
                  className="text-[11px] text-text-dim hover:text-brand"
                >
                  {t("users.view_budget", "Budget →")}
                </Link>
                <Link
                  to="/users/$name/policy"
                  params={{ name: u.name }}
                  className="text-[11px] text-text-dim hover:text-brand"
                >
                  {t("users.view_policy", "Permissions →")}
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

      {/* Delete confirm */}
      <Modal
        isOpen={confirmDelete !== null}
        onClose={() => setConfirmDelete(null)}
        title={t("users.confirm_delete_title", "Delete user?")}
        size="sm"
      >
        {confirmDelete ? (
          <div className="space-y-4">
            <p className="text-sm text-text-dim">
              {t(
                "users.confirm_delete_body",
                "This removes the user from config.toml and rebuilds the RBAC channel index. Any platform identity that mapped to this user will fall through to the default-deny path.",
              )}
            </p>
            <p className="text-sm font-mono">{confirmDelete.name}</p>
            <div className="flex gap-2 justify-end">
              <Button
                variant="secondary"
                onClick={() => setConfirmDelete(null)}
              >
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                variant="danger"
                isLoading={deleteMut.isPending}
                onClick={async () => {
                  await deleteMut.mutateAsync(confirmDelete.name);
                  setConfirmDelete(null);
                }}
              >
                {t("common.delete", "Delete")}
              </Button>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
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
          options={ROLES.map(r => ({ value: r, label: r }))}
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
                      label: p.label,
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
  const [step, setStep] = useState(0);
  const [channel, setChannel] = useState<string>("telegram");
  const [platformId, setPlatformId] = useState("");
  const [error, setError] = useState<string | null>(null);
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
    setAcknowledged(false);
  } else if (!user) {
    lastUser.current = null;
  }

  const tile = PLATFORM_TILES.find(p => p.key === channel);

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
          <ol className="flex items-center gap-2 text-[10px] uppercase tracking-widest text-text-dim">
            {[
              t("users.wizard_step1", "User"),
              t("users.wizard_step2", "Platform"),
              t("users.wizard_step3", "Identifier"),
              t("users.wizard_step4", "Confirm"),
            ].map((label, i) => (
              <li
                key={label}
                className={`flex items-center gap-1 ${
                  i === step ? "text-brand font-bold" : ""
                }`}
              >
                <span
                  className={`w-5 h-5 rounded-full text-[10px] flex items-center justify-center ${
                    i <= step
                      ? "bg-brand/20 text-brand"
                      : "bg-main/30 text-text-dim"
                  }`}
                >
                  {i + 1}
                </span>
                {label}
                {i < 3 ? <span className="opacity-30">›</span> : null}
              </li>
            ))}
          </ol>

          {step === 0 ? (
            <div className="space-y-2">
              <p className="text-xs text-text-dim">
                {t(
                  "users.wizard_user_desc",
                  "We'll add a binding to this user. The wizard never creates new users — pick a different user from the list to retarget.",
                )}
              </p>
              <Card padding="md">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-bold">{user.name}</span>
                  <Badge variant={roleVariant(user.role)}>{user.role}</Badge>
                </div>
                <p className="mt-1 text-[11px] text-text-dim">
                  {Object.keys(user.channel_bindings).length}{" "}
                  {t("users.bindings_suffix", "channel binding(s)")}
                </p>
              </Card>
            </div>
          ) : null}

          {step === 1 ? (
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
                    <p className="text-sm font-bold">{p.label}</p>
                    <p className="mt-1 text-[11px] text-text-dim">{p.hint}</p>
                  </button>
                ))}
              </div>
            </div>
          ) : null}

          {step === 2 ? (
            <div className="space-y-2">
              <p className="text-xs text-text-dim">
                {t(
                  "users.wizard_id_desc",
                  "Paste the platform's identifier. The format hint matches the channel you picked.",
                )}
              </p>
              {tile ? (
                <Card padding="md">
                  <p className="text-sm font-bold">{tile.label}</p>
                  <p className="mt-1 text-[11px] text-text-dim">{tile.hint}</p>
                  <p className="mt-2 text-[11px] font-mono text-text-dim">
                    {t("users.wizard_example", "Example")}: {tile.example}
                  </p>
                </Card>
              ) : null}
              <Input
                label={t("users.wizard_id_label", "platform_id")}
                value={platformId}
                onChange={e => setPlatformId(e.target.value)}
                placeholder={tile?.example}
              />
            </div>
          ) : null}

          {step === 3 ? (
            <div className="space-y-2">
              <p className="text-xs text-text-dim">
                {t(
                  "users.wizard_confirm_desc",
                  "Confirm and write to config.toml. The kernel rebuilds its RBAC channel index in place — no restart needed.",
                )}
              </p>
              <Card padding="md">
                <p className="text-xs">
                  <span className="text-text-dim">{t("users.user", "User")}: </span>
                  <span className="font-bold">{user.name}</span>
                </p>
                <p className="mt-1 text-xs">
                  <span className="text-text-dim">
                    {t("users.wizard_platform", "Platform")}:{" "}
                  </span>
                  <span className="font-bold">{channel}</span>
                </p>
                <p className="mt-1 text-xs font-mono">
                  <span className="text-text-dim">platform_id: </span>
                  {platformId || (
                    <span className="opacity-50">— missing —</span>
                  )}
                </p>
              </Card>

              {/* Ownership warning — there is currently no automated
                  challenge/response over the channel bot, so the platform_id
                  is taken on faith. Surface that risk so an Owner can't
                  silently bind another user's id. */}
              <div className="rounded-xl border border-warning/40 bg-warning/10 p-3 text-xs space-y-2">
                <p className="font-bold text-warning">
                  {t(
                    "users.wizard_unverified_title",
                    "No automated ownership check",
                  )}
                </p>
                <p className="text-text-dim">
                  {t(
                    "users.wizard_unverified_body",
                    "LibreFang does not yet ping the platform to confirm this id belongs to {{user}}. Anyone with Owner rights can bind any id to any user row, which silently retargets future RBAC and rate-limit decisions. Verify the platform_id with the user out-of-band before saving.",
                    { user: user.name },
                  )}
                </p>
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
            {step < 3 ? (
              <Button
                variant="primary"
                onClick={() => {
                  if (step === 2 && !platformId.trim()) {
                    setError(
                      t("users.err_id_required", "platform_id is required."),
                    );
                    return;
                  }
                  setError(null);
                  setStep(s => Math.min(3, s + 1));
                }}
              >
                {t("common.next", "Next")}
              </Button>
            ) : (
              <Button
                variant="primary"
                isLoading={busy}
                disabled={!acknowledged}
                title={
                  !acknowledged
                    ? t(
                        "users.wizard_ack_required",
                        "Acknowledge the ownership warning to save.",
                      )
                    : undefined
                }
                onClick={async () => {
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
            placeholder="name,role,telegram,discord\nalice,admin,123,\nbob,user,,456"
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
                  <Badge variant={roleVariant(r.role)}>{r.role}</Badge>
                  <span className="text-text-dim">
                    {Object.keys(r.channel_bindings ?? {}).length} bindings
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
              {result.created} created · {result.updated} updated ·{" "}
              {result.failed} failed
            </p>
            {result.rows.some(r => r.error) ? (
              <ul className="mt-2 space-y-0.5 text-[11px] text-error">
                {result.rows
                  .filter(r => r.error)
                  .map(r => (
                    <li key={r.index}>
                      row {r.index + 1} ({r.name}): {r.error}
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
  return (
    <div
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
      className={`cursor-pointer rounded-xl border-2 border-dashed p-6 text-center text-xs transition-colors ${
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
    </div>
  );
}

// Tiny CSV parser tuned for the import shape: header row + simple rows. We
// don't pull in a full CSV library because the dashboard ships zero-bundle
// hot paths and the import body shape is a documented narrow contract.
// CSV parsing now lives in `lib/csvParser.ts` so it can be unit-tested for
// quoted-newline + BOM handling without dragging in React.
