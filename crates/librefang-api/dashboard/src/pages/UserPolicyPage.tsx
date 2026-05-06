// Per-user permission matrix editor (RBAC M3 / #3205, M6 follow-up).
//
// Reads `/api/users/{name}/policy` via `usePermissionPolicy`, edits the
// four slots independently (tool allow/deny, tool categories, memory
// access, channel rules), and PUTs the whole sheet back through
// `useUpdateUserPolicy`. Validation mirrors the daemon's checks so the
// user sees errors inline before a round-trip.

import { useCallback, useEffect, useMemo, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useParams, Link } from "@tanstack/react-router";
import {
  ListChecks,
  ArrowLeft,
  Save,
  AlertTriangle,
  Plus,
  X,
} from "lucide-react";

import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";
import { usePermissionPolicy } from "../lib/queries/permissionPolicy";
import { useUpdateUserPolicy } from "../lib/mutations/users";
import type {
  PermissionPolicy,
  PermissionPolicyUpdate,
  ChannelToolPolicy,
} from "../lib/http/client";

// Channels we expose by default in the per-channel rules table. Operators
// can still PUT additional keys via the API; this just keeps the form
// approachable.
const DEFAULT_CHANNELS = ["telegram", "discord", "slack", "email"] as const;

interface FormState {
  tool_policy: { allowed: string; denied: string; enabled: boolean };
  tool_categories: { allowed: string; denied: string; enabled: boolean };
  memory_access: {
    enabled: boolean;
    readable: string;
    writable: string;
    pii_access: boolean;
    export_allowed: boolean;
    delete_allowed: boolean;
  };
  channel_tool_rules: Record<string, { allowed: string; denied: string }>;
}

// Newline-separated textarea contents <-> string[]. Trim each line; drop
// blanks. Mirrors the server's `validate_string_list` behaviour so the
// preview the user types matches what the server will accept.
function parseList(raw: string): string[] {
  return raw
    .split("\n")
    .map(line => line.trim())
    .filter(line => line.length > 0);
}

function formatList(items: string[] | undefined): string {
  if (!items || items.length === 0) return "";
  return items.join("\n");
}

function policyToForm(policy: PermissionPolicy | undefined): FormState {
  return {
    tool_policy: {
      enabled: !!policy?.tool_policy,
      allowed: formatList(policy?.tool_policy?.allowed_tools),
      denied: formatList(policy?.tool_policy?.denied_tools),
    },
    tool_categories: {
      enabled: !!policy?.tool_categories,
      allowed: formatList(policy?.tool_categories?.allowed_groups),
      denied: formatList(policy?.tool_categories?.denied_groups),
    },
    memory_access: {
      enabled: !!policy?.memory_access,
      readable: formatList(policy?.memory_access?.readable_namespaces),
      writable: formatList(policy?.memory_access?.writable_namespaces),
      pii_access: policy?.memory_access?.pii_access ?? false,
      export_allowed: policy?.memory_access?.export_allowed ?? false,
      delete_allowed: policy?.memory_access?.delete_allowed ?? false,
    },
    channel_tool_rules: (() => {
      const out: Record<string, { allowed: string; denied: string }> = {};
      for (const ch of DEFAULT_CHANNELS) {
        out[ch] = { allowed: "", denied: "" };
      }
      for (const [ch, rule] of Object.entries(policy?.channel_tool_rules ?? {})) {
        out[ch] = {
          allowed: formatList(rule.allowed_tools),
          denied: formatList(rule.denied_tools),
        };
      }
      return out;
    })(),
  };
}

// Mirror of the daemon's validators in `routes/users.rs`. We surface
// errors inline before the PUT round-trip so a typo doesn't waste a
// request, but the daemon revalidates so this layer is convenience only.
type ValidatorT = (key: string, fallback: string, vars?: Record<string, unknown>) => string;

function validateForm(form: FormState, t: ValidatorT): string | null {
  const checkList = (label: string, items: string[]): string | null => {
    const seen = new Set<string>();
    for (const item of items) {
      if (item.length === 0) {
        return t(
          "userPolicy.errors.empty_entry",
          "{{field}} contains an empty entry",
          { field: label },
        );
      }
      if (seen.has(item)) {
        return t(
          "userPolicy.errors.duplicate_entry",
          "{{field}} contains duplicate entry '{{item}}'",
          { field: label, item },
        );
      }
      seen.add(item);
    }
    return null;
  };

  if (form.tool_policy.enabled) {
    const allowed = parseList(form.tool_policy.allowed);
    const denied = parseList(form.tool_policy.denied);
    const e =
      checkList(t("tool_policy.allowed_tools", "Allowed tools"), allowed) ??
      checkList(t("tool_policy.denied_tools", "Denied tools"), denied);
    if (e) return e;
  }
  if (form.tool_categories.enabled) {
    const allowed = parseList(form.tool_categories.allowed);
    const denied = parseList(form.tool_categories.denied);
    const e =
      checkList(t("tool_categories.allowed_groups", "Allowed groups"), allowed) ??
      checkList(t("tool_categories.denied_groups", "Denied groups"), denied);
    if (e) return e;
  }
  if (form.memory_access.enabled) {
    const readable = parseList(form.memory_access.readable);
    const writable = parseList(form.memory_access.writable);
    const e =
      checkList(
        t("memory_access.readable_namespaces", "Readable namespaces"),
        readable,
      ) ??
      checkList(
        t("memory_access.writable_namespaces", "Writable namespaces"),
        writable,
      );
    if (e) return e;
    for (const w of writable) {
      if (!readable.includes(w)) {
        return t(
          "userPolicy.errors.writable_not_subset",
          "memory_access.writable_namespaces['{{ns}}'] is not in readable_namespaces (writable must be a subset of readable)",
          { ns: w },
        );
      }
    }
  }
  for (const [ch, rule] of Object.entries(form.channel_tool_rules)) {
    const allowed = parseList(rule.allowed);
    const denied = parseList(rule.denied);
    const e =
      checkList(`channel_tool_rules['${ch}'].allowed_tools`, allowed) ??
      checkList(`channel_tool_rules['${ch}'].denied_tools`, denied);
    if (e) return e;
  }
  return null;
}

function formToPayload(form: FormState): PermissionPolicyUpdate {
  const payload: PermissionPolicyUpdate = {};
  payload.tool_policy = form.tool_policy.enabled
    ? {
        allowed_tools: parseList(form.tool_policy.allowed),
        denied_tools: parseList(form.tool_policy.denied),
      }
    : null;
  payload.tool_categories = form.tool_categories.enabled
    ? {
        allowed_groups: parseList(form.tool_categories.allowed),
        denied_groups: parseList(form.tool_categories.denied),
      }
    : null;
  payload.memory_access = form.memory_access.enabled
    ? {
        readable_namespaces: parseList(form.memory_access.readable),
        writable_namespaces: parseList(form.memory_access.writable),
        pii_access: form.memory_access.pii_access,
        export_allowed: form.memory_access.export_allowed,
        delete_allowed: form.memory_access.delete_allowed,
      }
    : null;
  // Build the channel map: omit channels that have no rules, so the
  // server-side empty-map = "preserve" semantic doesn't kick in. We pass
  // an explicit object so PUT becomes a full replace of the channel slot.
  const channelRules: Record<string, ChannelToolPolicy> = {};
  for (const [ch, rule] of Object.entries(form.channel_tool_rules)) {
    const allowed = parseList(rule.allowed);
    const denied = parseList(rule.denied);
    if (allowed.length > 0 || denied.length > 0) {
      channelRules[ch] = { allowed_tools: allowed, denied_tools: denied };
    }
  }
  payload.channel_tool_rules = channelRules;
  return payload;
}

// Loose channel-key sanity check — kernel side accepts arbitrary strings,
// but typos like " Telegram " or empty are almost always wrong. We trim,
// lowercase, and require at least one printable non-space char so the
// resulting key matches what the channel adapter actually emits.
function normalizeChannelKey(raw: string): string {
  return raw.trim().toLowerCase();
}


function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    return false;
  }
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    if (!Object.prototype.hasOwnProperty.call(b, key)) return false;
    if (!deepEqual((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key])) {
      return false;
    }
  }
  return true;
}

export function UserPolicyPage() {
  const { t } = useTranslation();
  const { name } = useParams({ from: "/users/$name/policy" });

  const policyQuery = usePermissionPolicy(name);
  const updateMutation = useUpdateUserPolicy();

  const [form, setForm] = useState<FormState>(() => policyToForm(undefined));
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitOk, setSubmitOk] = useState(false);
  // Inline add-channel state. Kept local to the page (not in the form)
  // because it's transient — the value disappears once the channel slot
  // joins `form.channel_tool_rules`.
  const [newChannel, setNewChannel] = useState("");
  const [newChannelError, setNewChannelError] = useState<string | null>(null);

  // Form mutation wrapper. Wrapping every setForm in this clears the
  // stale "Policy saved." card on the first edit after a successful
  // save — keeps the success state from shadowing fresh unsaved work.
  // Cheaper than a useEffect-with-form-hash; we already touch every
  // setter on edit.
  const editForm = useCallback(
    (mutator: (prev: FormState) => FormState) => {
      setForm(mutator);
      if (submitOk) setSubmitOk(false);
    },
    [submitOk],
  );

  // Re-hydrate the form whenever the underlying query resolves a new value
  // (e.g. on initial load or after invalidation).
  useEffect(() => {
    if (policyQuery.data) {
      setForm(policyToForm(policyQuery.data));
    }
  }, [policyQuery.data]);

  // Track whether the form differs from the last loaded server state so
  // we can enable / disable Discard and surface "Unsaved changes" hint.
  const isDirty = useMemo(() => {
    if (!policyQuery.data) return false;
    return !deepEqual(form, policyToForm(policyQuery.data));
  }, [form, policyQuery.data]);

  // Discard: re-seed straight from the last query payload. We keep
  // submitError / submitOk untouched so the user still sees the
  // outcome of their last submit attempt.
  const handleDiscard = useCallback(() => {
    if (!policyQuery.data) return;
    setForm(policyToForm(policyQuery.data));
    setNewChannel("");
    setNewChannelError(null);
  }, [policyQuery.data]);

  const validationError = useMemo(
    () =>
      validateForm(form, (key, fallback, vars) =>
        t(key, fallback, vars as Record<string, unknown> | undefined) as string,
      ),
    [form, t],
  );

  const handleAddChannel = () => {
    const key = normalizeChannelKey(newChannel);
    if (!key) {
      setNewChannelError(
        t("user_policy.add_channel_err_empty", "Channel name is required."),
      );
      return;
    }
    if (Object.prototype.hasOwnProperty.call(form.channel_tool_rules, key)) {
      const tmpl = t(
        "user_policy.add_channel_err_duplicate",
        "Channel '{{key}}' already has a rule slot.",
      );
      setNewChannelError(tmpl.replace("{{key}}", () => key));
      return;
    }
    editForm(f => ({
      ...f,
      channel_tool_rules: {
        ...f.channel_tool_rules,
        [key]: { allowed: "", denied: "" },
      },
    }));
    setNewChannel("");
    setNewChannelError(null);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitError(null);
    setSubmitOk(false);
    if (validationError) {
      setSubmitError(validationError);
      return;
    }
    try {
      await updateMutation.mutateAsync({
        name,
        policy: formToPayload(form),
      });
      setSubmitOk(true);
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : String(err));
    }
  };

  if (policyQuery.isLoading) {
    return (
      <div className="flex flex-col gap-6">
        <PageHeader
          icon={<ListChecks className="h-4 w-4" />}
          title={t("user_policy.title", "Permission matrix")}
          subtitle={name}
          helpText={t("user_policy.help")}
        />
        <CardSkeleton />
      </div>
    );
  }

  if (policyQuery.isError) {
    return (
      <div className="flex flex-col gap-6">
        <PageHeader
          icon={<ListChecks className="h-4 w-4" />}
          title={t("user_policy.title", "Permission matrix")}
          subtitle={name}
          helpText={t("user_policy.help")}
        />
        <EmptyState
          icon={<AlertTriangle className="h-6 w-6" />}
          title={t("user_policy.load_error_title", "Failed to load policy")}
          description={
            policyQuery.error instanceof Error
              ? policyQuery.error.message
              : t(
                  "user_policy.load_error_body",
                  "The daemon returned an error fetching the per-user policy slice.",
                )
          }
        />
      </div>
    );
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-6">
      <PageHeader
        icon={<ListChecks className="h-4 w-4" />}
        title={t("user_policy.title", "Permission matrix")}
        subtitle={name}
        helpText={t("user_policy.help")}
        actions={
          <div className="flex items-center gap-3">
            <Link
              to="/users"
              className="inline-flex items-center gap-1.5 text-xs text-text-dim hover:text-brand"
            >
              <ArrowLeft className="h-3.5 w-3.5" />
              {t("user_policy.back", "Back to users")}
            </Link>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              disabled={updateMutation.isPending || !!validationError}
            >
              <Save className="h-3.5 w-3.5" />
              {t("user_policy.save", "Save")}
            </Button>
          </div>
        }
      />

      {submitError && (
        <Card padding="md">
          <div className="flex items-start gap-2 text-sm text-error">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span>{submitError}</span>
          </div>
        </Card>
      )}
      {submitOk && !submitError && (
        <Card padding="md">
          <div className="text-sm font-bold text-success">
            {t("user_policy.saved", "Policy saved.")}
          </div>
        </Card>
      )}

      <Card padding="lg">
        <SectionHeader
          title={t("user_policy.section_tool_policy", "Tool allow / deny")}
          description={t(
            "user_policy.tool_policy_desc",
            "Per-user allow + deny patterns. Layered ON TOP of agent + channel rules. Deny wins.",
          )}
          enabled={form.tool_policy.enabled}
          onToggle={enabled =>
            editForm(f => ({
              ...f,
              tool_policy: { ...f.tool_policy, enabled },
            }))
          }
        />
        {form.tool_policy.enabled && (
          <div className="mt-4 grid gap-4 md:grid-cols-2">
            <Textarea
              label={t("user_policy.allowed_tools", "Allowed tools")}
              hint={t(
                "user_policy.glob_hint",
                "One pattern per line. Glob with `*` allowed.",
              )}
              value={form.tool_policy.allowed}
              onChange={value =>
                editForm(f => ({
                  ...f,
                  tool_policy: { ...f.tool_policy, allowed: value },
                }))
              }
            />
            <Textarea
              label={t("user_policy.denied_tools", "Denied tools")}
              hint={t("user_policy.glob_hint_deny", "Always wins over allow.")}
              value={form.tool_policy.denied}
              onChange={value =>
                editForm(f => ({
                  ...f,
                  tool_policy: { ...f.tool_policy, denied: value },
                }))
              }
            />
          </div>
        )}
      </Card>

      <Card padding="lg">
        <SectionHeader
          title={t("user_policy.section_categories", "Tool categories")}
          description={t(
            "user_policy.categories_desc",
            "Bulk allow / deny by tool group name (defined in `KernelConfig.tool_policy.groups`).",
          )}
          enabled={form.tool_categories.enabled}
          onToggle={enabled =>
            editForm(f => ({
              ...f,
              tool_categories: { ...f.tool_categories, enabled },
            }))
          }
        />
        {form.tool_categories.enabled && (
          <div className="mt-4 grid gap-4 md:grid-cols-2">
            <Textarea
              label={t("user_policy.allowed_groups", "Allowed groups")}
              hint={t(
                "user_policy.group_hint",
                "One group name per line (e.g. `web_tools`).",
              )}
              value={form.tool_categories.allowed}
              onChange={value =>
                editForm(f => ({
                  ...f,
                  tool_categories: { ...f.tool_categories, allowed: value },
                }))
              }
            />
            <Textarea
              label={t("user_policy.denied_groups", "Denied groups")}
              hint={t("user_policy.glob_hint_deny", "Always wins over allow.")}
              value={form.tool_categories.denied}
              onChange={value =>
                editForm(f => ({
                  ...f,
                  tool_categories: { ...f.tool_categories, denied: value },
                }))
              }
            />
          </div>
        )}
      </Card>

      <Card padding="lg">
        <SectionHeader
          title={t("user_policy.section_memory", "Memory access")}
          description={t(
            "user_policy.memory_desc",
            "Namespace ACL + PII redaction toggles. Writable must be a subset of readable.",
          )}
          enabled={form.memory_access.enabled}
          onToggle={enabled =>
            editForm(f => ({
              ...f,
              memory_access: { ...f.memory_access, enabled },
            }))
          }
        />
        {form.memory_access.enabled && (
          <div className="mt-4 flex flex-col gap-4">
            <div className="grid gap-4 md:grid-cols-2">
              <Textarea
                label={t("user_policy.readable_ns", "Readable namespaces")}
                hint={t(
                  "user_policy.ns_hint",
                  "One namespace per line. `*` matches all.",
                )}
                value={form.memory_access.readable}
                onChange={value =>
                  editForm(f => ({
                    ...f,
                    memory_access: { ...f.memory_access, readable: value },
                  }))
                }
              />
              <Textarea
                label={t("user_policy.writable_ns", "Writable namespaces")}
                hint={t(
                  "user_policy.writable_hint",
                  "Must be a subset of readable.",
                )}
                value={form.memory_access.writable}
                onChange={value =>
                  editForm(f => ({
                    ...f,
                    memory_access: { ...f.memory_access, writable: value },
                  }))
                }
              />
            </div>
            <div className="flex flex-wrap gap-4">
              <CheckboxLabel
                label={t("user_policy.pii_access", "PII access")}
                checked={form.memory_access.pii_access}
                onChange={checked =>
                  editForm(f => ({
                    ...f,
                    memory_access: { ...f.memory_access, pii_access: checked },
                  }))
                }
              />
              <CheckboxLabel
                label={t("user_policy.export_allowed", "Export allowed")}
                checked={form.memory_access.export_allowed}
                onChange={checked =>
                  editForm(f => ({
                    ...f,
                    memory_access: {
                      ...f.memory_access,
                      export_allowed: checked,
                    },
                  }))
                }
              />
              <CheckboxLabel
                label={t("user_policy.delete_allowed", "Delete allowed")}
                checked={form.memory_access.delete_allowed}
                onChange={checked =>
                  editForm(f => ({
                    ...f,
                    memory_access: {
                      ...f.memory_access,
                      delete_allowed: checked,
                    },
                  }))
                }
              />
            </div>
          </div>
        )}
      </Card>

      <Card padding="lg">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-bold">
              {t("user_policy.section_channels", "Per-channel tool rules")}
            </p>
            <p className="mt-1 text-xs text-text-dim">
              {t(
                "user_policy.channels_desc",
                "Override the user's tool surface per channel adapter (telegram / discord / …). Add custom adapters with the input below.",
              )}
            </p>
          </div>
          <Badge variant="info">{Object.keys(form.channel_tool_rules).length}</Badge>
        </div>
        <div className="mt-4 flex flex-col gap-4">
          {Object.entries(form.channel_tool_rules).map(([ch, rule]) => {
            const isDefault = (DEFAULT_CHANNELS as readonly string[]).includes(ch);
            return (
              <div key={ch} className="rounded-xl border border-border-subtle p-3">
                <div className="flex items-center justify-between gap-2">
                  <div className="text-xs font-black uppercase tracking-widest text-text-dim">
                    {ch}
                    {!isDefault ? (
                      <span className="ml-2 normal-case tracking-normal text-[10px] text-text-dim/60">
                        ({t("user_policy.custom_channel", "custom")})
                      </span>
                    ) : null}
                  </div>
                  {!isDefault ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      aria-label={t("user_policy.remove_channel", "Remove channel")}
                      onClick={() =>
                        editForm(f => {
                          const next = { ...f.channel_tool_rules };
                          delete next[ch];
                          return { ...f, channel_tool_rules: next };
                        })
                      }
                    >
                      <X className="h-3 w-3" />
                    </Button>
                  ) : null}
                </div>
                <div className="mt-3 grid gap-3 md:grid-cols-2">
                  <Textarea
                    label={t("user_policy.allowed_tools", "Allowed tools")}
                    value={rule.allowed}
                    onChange={value =>
                      editForm(f => ({
                        ...f,
                        channel_tool_rules: {
                          ...f.channel_tool_rules,
                          [ch]: { ...f.channel_tool_rules[ch], allowed: value },
                        },
                      }))
                    }
                  />
                  <Textarea
                    label={t("user_policy.denied_tools", "Denied tools")}
                    value={rule.denied}
                    onChange={value =>
                      editForm(f => ({
                        ...f,
                        channel_tool_rules: {
                          ...f.channel_tool_rules,
                          [ch]: { ...f.channel_tool_rules[ch], denied: value },
                        },
                      }))
                    }
                  />
                </div>
              </div>
            );
          })}
        </div>
        {/* Add-channel inline form. Lets operators surface a custom channel
            adapter (e.g. `wechat`, `matrix`) without hand-editing config.toml.
            Validation here mirrors the obvious mistakes — the daemon still
            accepts any non-empty string. */}
        <div className="mt-4 border-t border-border-subtle pt-4">
          <label className="block text-[10px] font-black uppercase tracking-widest text-text-dim">
            {t("user_policy.add_channel_label", "Add channel")}
          </label>
          <div className="mt-2 flex gap-2 items-start">
            <div className="grow">
              <Input
                value={newChannel}
                placeholder={t(
                  "user_policy.add_channel_placeholder",
                  "wechat, matrix, …",
                )}
                onChange={e => {
                  setNewChannel(e.target.value);
                  if (newChannelError) setNewChannelError(null);
                }}
                onKeyDown={e => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    handleAddChannel();
                  }
                }}
              />
            </div>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={handleAddChannel}
              leftIcon={<Plus className="h-3.5 w-3.5" />}
            >
              {t("common.add", "Add")}
            </Button>
          </div>
          {newChannelError ? (
            <p className="mt-1.5 text-[11px] text-error">{newChannelError}</p>
          ) : (
            <p className="mt-1.5 text-[11px] text-text-dim">
              {t(
                "user_policy.add_channel_hint",
                "Channel adapter name (lowercase). Must match the key the bridge emits.",
              )}
            </p>
          )}
        </div>
      </Card>

      {validationError && (
        <Card padding="md">
          <div className="flex items-start gap-2 text-sm text-warning">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span>{validationError}</span>
          </div>
        </Card>
      )}

      {/* Sticky save bar — same Save action as the PageHeader, but always
          in reach when the form is long. Lives inside the <form> so the
          Save button still triggers `onSubmit` natively. Discard reverts
          to the last server-loaded state without an extra round-trip. */}
      <div className="sticky bottom-0 z-10 -mx-4 sm:-mx-6 mt-2 border-t border-border-subtle bg-surface/95 px-4 sm:px-6 py-3 backdrop-blur supports-[backdrop-filter]:bg-surface/80">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-[11px] text-text-dim">
            {validationError
              ? t(
                  "userPolicy.save_disabled_invalid",
                  "Fix validation errors above to enable Save",
                )
              : isDirty
                ? t("userPolicy.unsaved_changes", "You have unsaved changes")
                : t("userPolicy.no_changes", "No unsaved changes")}
          </p>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={handleDiscard}
              disabled={!isDirty || updateMutation.isPending}
            >
              {t("userPolicy.discard", "Discard changes")}
            </Button>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              disabled={updateMutation.isPending || !!validationError || !isDirty}
              title={
                validationError
                  ? t(
                      "userPolicy.save_disabled_invalid",
                      "Fix validation errors above to enable Save",
                    )
                  : !isDirty
                    ? t("userPolicy.no_changes", "No unsaved changes")
                    : undefined
              }
            >
              <Save className="h-3.5 w-3.5" />
              {t("user_policy.save", "Save")}
            </Button>
          </div>
        </div>
      </div>
    </form>
  );
}

interface SectionHeaderProps {
  title: string;
  description: string;
  enabled: boolean;
  onToggle: (enabled: boolean) => void;
}

function SectionHeader({ title, description, enabled, onToggle }: SectionHeaderProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <p className="text-sm font-bold">{title}</p>
        <p className="mt-1 text-xs text-text-dim">{description}</p>
      </div>
      <CheckboxLabel
        label={
          enabled
            ? t("userPolicy.toggle.configured", "Configured")
            : t("userPolicy.toggle.not_set", "Not set")
        }
        checked={enabled}
        onChange={onToggle}
      />
    </div>
  );
}

interface TextareaProps {
  label: string;
  hint?: string;
  value: string;
  onChange: (value: string) => void;
}

function Textarea({ label, hint, value, onChange }: TextareaProps) {
  const reactId = useId();
  const id = `${reactId}-${label.toLowerCase().replace(/\s+/g, "-")}`;
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-[10px] font-black uppercase tracking-widest text-text-dim">
        {label}
      </label>
      <textarea
        id={id}
        className="w-full min-h-[100px] rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:outline-none focus:ring-2 focus:ring-brand/40"
        value={value}
        onChange={e => onChange(e.target.value)}
      />
      {hint && <p className="text-[11px] text-text-dim">{hint}</p>}
    </div>
  );
}

interface CheckboxLabelProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}

function CheckboxLabel({ label, checked, onChange }: CheckboxLabelProps) {
  return (
    <label className="inline-flex items-center gap-2 text-xs font-medium text-text-main cursor-pointer">
      <input
        type="checkbox"
        checked={checked}
        onChange={e => onChange(e.target.checked)}
        className="h-4 w-4 rounded border-border-subtle accent-brand"
      />
      {label}
    </label>
  );
}

