// Groups page (#7745).
//
// Surfaces:
//   - List view with name / description / member / role search
//   - Create + edit modal (name, description, roles)
//   - Membership editor: add a user by picking from the existing user list,
//     remove one inline
//   - Delete confirmation
//
// Membership is FLAT — there is no parent/child picker because groups do not
// nest. The rationale lives on `GroupConfig` in `librefang-types`; the short
// form is that both consumers of this entity want flattened effective
// membership, and an external identity provider hands us exactly that.
//
// All API access lives in `lib/queries/groups.ts` and
// `lib/mutations/groups.ts`. This file only renders.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Users,
  UserCheck,
  Plus,
  Search,
  X,
  Shield,
  Tag,
  Trash2,
  AlertTriangle,
} from "lucide-react";

import type { GroupItem, GroupUpsertPayload } from "../lib/http/client";
import { useGroups } from "../lib/queries/groups";
import { useUsers } from "../lib/queries/users";
import {
  useAddGroupMember,
  useCreateGroup,
  useDeleteGroup,
  useRemoveGroupMember,
  useUpdateGroup,
} from "../lib/mutations/groups";
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
import { StaggerList } from "../components/ui/StaggerList";

function errorMessage(err: unknown, fallback: string): string {
  return err instanceof Error && err.message ? err.message : fallback;
}

export function GroupsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore(s => s.addToast);

  // ── state ────────────────────────────────────────────────────────────
  const [search, setSearch] = useState("");
  const [editing, setEditing] = useState<GroupItem | null>(null);
  const [creating, setCreating] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<GroupItem | null>(null);
  // Per-group "add member" selection. Keyed by group name so two expanded
  // rows do not fight over one piece of state.
  const [memberDraft, setMemberDraft] = useState<Record<string, string>>({});

  // ── data ─────────────────────────────────────────────────────────────
  const groupsQuery = useGroups({ search });
  // The user list backs the add-member picker. Choosing from the registered
  // users is the common case; a name that has no user row yet can still be a
  // member (the daemon accepts it), it just cannot be picked here.
  const usersQuery = useUsers();

  const createMut = useCreateGroup();
  const updateMut = useUpdateGroup();
  const deleteMut = useDeleteGroup();
  const addMemberMut = useAddGroupMember();
  const removeMemberMut = useRemoveGroupMember();

  const groups = groupsQuery.data ?? [];
  const users = usersQuery.data ?? [];

  const handleRefresh = () => {
    void groupsQuery.refetch();
  };

  async function handleSubmit(payload: GroupUpsertPayload) {
    try {
      if (editing) {
        await updateMut.mutateAsync({ originalName: editing.name, payload });
        addToast(t("groups.updated", "Group updated"), "success");
      } else {
        await createMut.mutateAsync(payload);
        addToast(t("groups.created", "Group created"), "success");
      }
      setEditing(null);
      setCreating(false);
    } catch (err) {
      addToast(errorMessage(err, t("groups.save_failed", "Could not save group")), "error");
    }
  }

  async function handleDelete(group: GroupItem) {
    try {
      await deleteMut.mutateAsync(group.name);
      addToast(t("groups.deleted", "Group deleted"), "success");
    } catch (err) {
      addToast(errorMessage(err, t("groups.delete_failed", "Could not delete group")), "error");
    } finally {
      setConfirmDelete(null);
    }
  }

  async function handleAddMember(group: GroupItem) {
    const user = memberDraft[group.name]?.trim();
    if (!user) return;
    try {
      await addMemberMut.mutateAsync({ group: group.name, user });
      setMemberDraft(d => ({ ...d, [group.name]: "" }));
    } catch (err) {
      addToast(errorMessage(err, t("groups.member_add_failed", "Could not add member")), "error");
    }
  }

  async function handleRemoveMember(group: GroupItem, user: string) {
    try {
      await removeMemberMut.mutateAsync({ group: group.name, user });
    } catch (err) {
      addToast(errorMessage(err, t("groups.member_remove_failed", "Could not remove member")), "error");
    }
  }

  // ── render ──────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Users className="h-4 w-4" />}
        title={t("groups.title", "Groups")}
        subtitle={t(
          "groups.subtitle",
          "Name a team instead of a person. Membership is many-to-many and flat — groups do not nest.",
        )}
        isFetching={groupsQuery.isFetching}
        onRefresh={handleRefresh}
        actions={
          <Button
            variant="primary"
            size="sm"
            leftIcon={<Plus className="h-3.5 w-3.5" />}
            onClick={() => setCreating(true)}
          >
            {t("groups.create", "New group")}
          </Button>
        }
        helpText={t(
          "groups.help",
          "A group's roles are conferred on every member, in the same role vocabulary channel bindings already use. Stored as [[groups]] in config.toml; writes are Owner-only.",
        )}
      />

      {/* Filter bar */}
      <Card padding="sm">
        <div className="flex flex-wrap gap-3 items-end">
          <div className="grow min-w-[220px]">
            <Input
              label={t("groups.search_label", "Search")}
              placeholder={t(
                "groups.search_placeholder",
                "Group, description, member or role…",
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
        </div>
      </Card>

      {/* List */}
      {groupsQuery.isPending ? (
        <StaggerList className="grid gap-4 md:grid-cols-2">
          <CardSkeleton />
          <CardSkeleton />
        </StaggerList>
      ) : groups.length === 0 ? (
        <EmptyState
          icon={<Users className="h-8 w-8" />}
          title={t("groups.empty_title", "No groups yet")}
          description={t(
            "groups.empty_desc",
            "Create a group for a rota, a shift, or a project team, then add the people currently filling it.",
          )}
        />
      ) : (
        <StaggerList className="grid gap-3 md:grid-cols-2">
          {groups.map(g => (
            <Card key={g.name} hover padding="md">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 grow">
                  <div className="flex items-center gap-2 flex-wrap">
                    <p className="text-sm font-bold truncate">{g.name}</p>
                    <Badge variant="info">
                      <UserCheck className="h-3 w-3 mr-1 inline" />
                      {t("groups.member_count", "{{n}} member(s)", {
                        n: g.member_count,
                      })}
                    </Badge>
                    {g.unknown_members.length > 0 ? (
                      <Badge
                        variant="warning"
                        title={t(
                          "groups.unknown_members_title",
                          "These members have no [[users]] entry yet. Accepted on purpose — an identity provider can name someone before they first sign in.",
                        )}
                      >
                        <AlertTriangle className="h-3 w-3 mr-1 inline" />
                        {t("groups.unknown_members_badge", "{{n}} unregistered", {
                          n: g.unknown_members.length,
                        })}
                      </Badge>
                    ) : null}
                  </div>

                  {g.description ? (
                    <p className="mt-1.5 text-[11px] text-text-dim">{g.description}</p>
                  ) : null}

                  {g.roles.length > 0 ? (
                    <ul className="mt-2 flex flex-wrap gap-1">
                      {g.roles.map(r => (
                        <li
                          key={r}
                          className="inline-flex items-center gap-1 font-mono text-[10px] rounded bg-main/40 px-1.5 py-0.5"
                        >
                          <Tag className="h-2.5 w-2.5" />
                          {r}
                        </li>
                      ))}
                    </ul>
                  ) : null}

                  {/* Membership editor */}
                  <div className="mt-3 border-t border-border-subtle pt-2">
                    {g.members.length > 0 ? (
                      <ul className="flex flex-wrap gap-1">
                        {g.members.map(m => (
                          <li
                            key={m}
                            className="inline-flex items-center gap-1 text-[10px] rounded bg-main/40 px-1.5 py-0.5"
                          >
                            <span
                              className={
                                g.unknown_members.includes(m)
                                  ? "font-mono text-warning"
                                  : "font-mono"
                              }
                              title={
                                g.unknown_members.includes(m)
                                  ? t(
                                      "groups.unknown_member_title",
                                      "No [[users]] entry for this name.",
                                    )
                                  : undefined
                              }
                            >
                              {m}
                            </span>
                            <button
                              type="button"
                              onClick={() => void handleRemoveMember(g, m)}
                              className="text-text-dim hover:text-error"
                              aria-label={t("groups.remove_member", "Remove member")}
                            >
                              <X className="h-3 w-3" />
                            </button>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="text-[11px] text-text-dim">
                        {t("groups.no_members", "No members yet.")}
                      </p>
                    )}

                    <div className="mt-2 flex items-end gap-2">
                      <div className="grow">
                        <Select
                          label={t("groups.add_member_label", "Add member")}
                          value={memberDraft[g.name] ?? ""}
                          options={[
                            {
                              value: "",
                              label: t("groups.pick_user", "Pick a user…"),
                            },
                            ...users
                              .filter(u => !g.members.includes(u.name))
                              .map(u => ({ value: u.name, label: u.name })),
                          ]}
                          onChange={e =>
                            setMemberDraft(d => ({ ...d, [g.name]: e.target.value }))
                          }
                        />
                      </div>
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={!memberDraft[g.name]}
                        onClick={() => void handleAddMember(g)}
                      >
                        {t("common.add", "Add")}
                      </Button>
                    </div>
                  </div>
                </div>

                <div className="flex flex-col gap-1.5 shrink-0 items-end">
                  <Button variant="ghost" size="sm" onClick={() => setEditing(g)}>
                    {t("common.edit", "Edit")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    leftIcon={<Trash2 className="h-3.5 w-3.5" />}
                    onClick={() => setConfirmDelete(g)}
                  >
                    {t("common.delete", "Delete")}
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </StaggerList>
      )}

      <GroupEditorModal
        isOpen={creating || editing !== null}
        group={editing}
        busy={createMut.isPending || updateMut.isPending}
        onClose={() => {
          setCreating(false);
          setEditing(null);
        }}
        onSubmit={handleSubmit}
      />

      <ConfirmDialog
        isOpen={confirmDelete !== null}
        title={t("groups.delete_title", "Delete group")}
        message={t(
          "groups.delete_message",
          "Deleting a group removes its membership with it. The users themselves are untouched.",
        )}
        tone="destructive"
        onConfirm={() => (confirmDelete ? handleDelete(confirmDelete) : undefined)}
        onClose={() => setConfirmDelete(null)}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Editor modal
// ---------------------------------------------------------------------------

function GroupEditorModal({
  isOpen,
  group,
  busy,
  onClose,
  onSubmit,
}: {
  isOpen: boolean;
  group: GroupItem | null;
  busy: boolean;
  onClose: () => void;
  onSubmit: (payload: GroupUpsertPayload) => void | Promise<void>;
}) {
  const { t } = useTranslation();
  // Re-key the form on the group identity so switching rows resets the fields
  // without an effect that races the open animation.
  const formKey = group ? `edit:${group.name}` : "create";
  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={group ? t("groups.edit_title", "Edit group") : t("groups.create", "New group")}
      size="lg"
    >
      <GroupEditorForm
        key={formKey}
        group={group}
        busy={busy}
        onCancel={onClose}
        onSubmit={onSubmit}
      />
    </Modal>
  );
}

function GroupEditorForm({
  group,
  busy,
  onCancel,
  onSubmit,
}: {
  group: GroupItem | null;
  busy: boolean;
  onCancel: () => void;
  onSubmit: (payload: GroupUpsertPayload) => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(group?.name ?? "");
  const [description, setDescription] = useState(group?.description ?? "");
  // Roles are edited as comma-separated text. The daemon trims, drops empties,
  // sorts and de-duplicates, so the client does not have to police the string.
  const [rolesText, setRolesText] = useState((group?.roles ?? []).join(", "));

  const roles = useMemo(
    () =>
      rolesText
        .split(",")
        .map(r => r.trim())
        .filter(Boolean),
    [rolesText],
  );

  const nameError = name.trim() ? undefined : t("groups.name_required", "Name is required");

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={e => {
        e.preventDefault();
        if (nameError) return;
        void onSubmit({
          name: name.trim(),
          description: description.trim(),
          // Membership is edited from the list row, not here — restating it
          // would make the modal clobber concurrent add/remove calls.
          members: group?.members ?? [],
          roles,
        });
      }}
    >
      <Input
        label={t("groups.name_label", "Name")}
        value={name}
        onChange={e => setName(e.target.value)}
        error={name.length > 0 ? nameError : undefined}
        placeholder="oncall"
      />
      <Input
        label={t("groups.description_label", "Description")}
        value={description}
        onChange={e => setDescription(e.target.value)}
        placeholder={t("groups.description_placeholder", "Support rota")}
      />
      <div>
        <Input
          label={t("groups.roles_label", "Roles (comma-separated)")}
          value={rolesText}
          onChange={e => setRolesText(e.target.value)}
          leftIcon={<Shield className="h-3.5 w-3.5" />}
          placeholder="approver, auditor"
        />
        <p className="mt-1 text-[11px] text-text-dim">
          {t(
            "groups.roles_help",
            "Conferred on every member. The group's own name is always conferred too, so it does not need to be repeated here.",
          )}
        </p>
      </div>

      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          {t("common.cancel", "Cancel")}
        </Button>
        <Button type="submit" variant="primary" size="sm" disabled={busy || !!nameError}>
          {t("common.save", "Save")}
        </Button>
      </div>
    </form>
  );
}
