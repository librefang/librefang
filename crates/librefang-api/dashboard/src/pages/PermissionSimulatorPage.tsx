// Permission simulator (RBAC follow-up to M3/M5/M6).
//
// Pick a user → call `/api/authz/effective/{name}` → render every RBAC
// input slice that contributes to that user's permissions. The endpoint
// returns the raw configured policy (NOT the per-call gate decision)
// because reproducing the four-layer intersection here would silently
// drift from the runtime gate path. Admins reading this page compose
// the result mentally; the gate path stays the source of truth.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Shield,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Wrench,
  Layers,
  Database,
  DollarSign,
  Radio,
  Link2,
} from "lucide-react";

import { useUsers } from "../lib/queries/users";
import { useEffectivePermissions } from "../lib/queries/authz";
import type {
  EffectiveBudget,
  EffectiveChannelToolPolicy,
  EffectiveMemoryAccess,
  EffectiveToolCategories,
  EffectiveToolPolicy,
} from "../lib/http/client";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Select } from "../components/ui/Select";
import { EmptyState } from "../components/ui/EmptyState";
import { Skeleton } from "../components/ui/Skeleton";

// Roles ordered weakest → strongest, mirrors `UserRole as u8` in
// `librefang_kernel::auth::UserRole`. The role-level allow/deny grid
// remains useful even on top of the new effective-permissions data —
// it answers a different question ("what can the role itself do?")
// than the per-user-policy slices below.
const ROLE_ORDER = ["viewer", "user", "admin", "owner"] as const;
type Role = (typeof ROLE_ORDER)[number];

const ACTIONS: Array<{
  id: string;
  label: string;
  required: Role;
  description: string;
}> = [
  {
    id: "ChatWithAgent",
    label: "Chat with agent",
    required: "user",
    description: "Send messages to a running agent.",
  },
  {
    id: "ViewConfig",
    label: "View configuration",
    required: "user",
    description: "Read kernel config (redacted secrets).",
  },
  {
    id: "ViewUsage",
    label: "View usage / billing",
    required: "admin",
    description: "Inspect token / cost dashboards.",
  },
  {
    id: "SpawnAgent",
    label: "Spawn agent",
    required: "admin",
    description: "Create and start a new agent process.",
  },
  {
    id: "KillAgent",
    label: "Kill agent",
    required: "admin",
    description: "Stop a running agent.",
  },
  {
    id: "InstallSkill",
    label: "Install skill",
    required: "admin",
    description: "Install ClawHub / Skillhub / local skills.",
  },
  {
    id: "ModifyConfig",
    label: "Modify configuration",
    required: "owner",
    description: "Write changes back to config.toml.",
  },
  {
    id: "ManageUsers",
    label: "Manage users",
    required: "owner",
    description: "Create / delete users and rebind identities.",
  },
];

function roleAllows(actor: Role, required: Role): boolean {
  return ROLE_ORDER.indexOf(actor) >= ROLE_ORDER.indexOf(required);
}

export function PermissionSimulatorPage() {
  const { t } = useTranslation();
  const usersQuery = useUsers();
  const [selectedName, setSelectedName] = useState<string>("");

  const users = usersQuery.data ?? [];
  const selected = useMemo(
    () => users.find(u => u.name === selectedName) ?? users[0],
    [users, selectedName],
  );
  const role = (selected?.role as Role) ?? "user";

  const effectiveQuery = useEffectivePermissions(selected?.name ?? "");
  const effective = effectiveQuery.data;

  // 404 from the daemon is a deterministic "user not present in
  // AuthManager"; surface it distinctly from a generic fetch error so
  // operators don't chase a network problem when the cause is a stale
  // user list.
  const notFound =
    effectiveQuery.isError &&
    /404|not found/i.test(String(effectiveQuery.error));

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Shield className="h-4 w-4" />}
        title={t("simulator.title", "Permission simulator")}
        subtitle={t(
          "simulator.subtitle",
          "Pick a user and see every RBAC input contributing to their permissions.",
        )}
        badge={t("simulator.badge", "Live")}
        helpText={t(
          "simulator.help",
          "Sections show RAW configured slices — not the per-call gate decision (the runtime gate intersects per-agent ToolPolicy, per-user tool_policy / tool_categories, and per-channel rules). Slices labelled \"Not configured\" defer to other layers.",
        )}
      />

      <Card padding="md">
        <Select
          label={t("simulator.user_label", "User")}
          value={selected?.name ?? ""}
          options={users.map(u => ({
            value: u.name,
            label: `${u.name} (${u.role})`,
          }))}
          onChange={e => setSelectedName(e.target.value)}
          disabled={users.length === 0}
          placeholder={t("simulator.choose_user", "Select a user…")}
        />
      </Card>

      {users.length === 0 ? (
        <EmptyState
          icon={<Shield className="h-8 w-8" />}
          title={t("simulator.empty_title", "No users to simulate")}
          description={t(
            "simulator.empty_desc",
            "Add a user from the Users page first.",
          )}
        />
      ) : !selected ? null : (
        <>
          <RoleMatrixCard role={role} selectedName={selected.name} t={t} />

          {effectiveQuery.isLoading ? (
            <Card padding="md">
              <Skeleton className="h-32 w-full" />
            </Card>
          ) : notFound ? (
            <EmptyState
              icon={<AlertTriangle className="h-8 w-8" />}
              title={t("simulator.not_found_title", "User not found")}
              description={t(
                "simulator.not_found_desc",
                "The selected user is not registered with the AuthManager. They may have been removed from config.toml since the user list cached.",
              )}
            />
          ) : effectiveQuery.isError ? (
            <EmptyState
              icon={<AlertTriangle className="h-8 w-8" />}
              title={t(
                "simulator.error_title",
                "Could not load effective permissions",
              )}
              description={String(effectiveQuery.error)}
            />
          ) : effective ? (
            <>
              <ToolPolicyCard policy={effective.tool_policy} t={t} />
              <ToolCategoriesCard categories={effective.tool_categories} t={t} />
              <MemoryAccessCard access={effective.memory_access} t={t} />
              <BudgetCard budget={effective.budget} t={t} />
              <ChannelRulesCard rules={effective.channel_tool_rules} t={t} />
              <ChannelBindingsCard bindings={effective.channel_bindings} t={t} />
            </>
          ) : null}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------
// Section components
// ---------------------------------------------------------------------

type Translate = (key: string, fallback: string) => string;

function SectionHeader({
  icon,
  title,
  configured,
}: {
  icon: React.ReactNode;
  title: string;
  configured: boolean;
}) {
  return (
    <div className="flex items-center justify-between mb-3">
      <div className="flex items-center gap-2">
        <span className="text-text-dim">{icon}</span>
        <p className="text-sm font-bold">{title}</p>
      </div>
      <Badge variant={configured ? "info" : "default"}>
        {configured ? "Configured" : "Not configured"}
      </Badge>
    </div>
  );
}

function PatternList({ items, empty }: { items: string[]; empty: string }) {
  if (items.length === 0) {
    return <p className="text-xs text-text-dim italic">{empty}</p>;
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map(p => (
        <code
          key={p}
          className="rounded-md border border-border-subtle bg-surface-2 px-1.5 py-0.5 text-[11px]"
        >
          {p}
        </code>
      ))}
    </div>
  );
}

function RoleMatrixCard({
  role,
  selectedName,
  t,
}: {
  role: Role;
  selectedName: string;
  t: Translate;
}) {
  return (
    <Card padding="md">
      <div className="flex items-center gap-2 mb-4">
        <p className="text-sm font-bold">{selectedName}</p>
        <Badge variant="info">{role}</Badge>
        <span className="text-[11px] text-text-dim ml-auto">
          {t("simulator.role_matrix_caption", "Role-level coarse permissions")}
        </span>
      </div>
      <div className="grid gap-2 md:grid-cols-2">
        {ACTIONS.map(a => {
          const allowed = roleAllows(role, a.required);
          return (
            <div
              key={a.id}
              className={`flex items-start gap-3 rounded-xl border p-3 ${
                allowed
                  ? "border-success/30 bg-success/5"
                  : "border-error/30 bg-error/5"
              }`}
            >
              <div className="shrink-0 pt-0.5">
                {allowed ? (
                  <CheckCircle2 className="h-4 w-4 text-success" />
                ) : (
                  <XCircle className="h-4 w-4 text-error" />
                )}
              </div>
              <div className="min-w-0">
                <p className="text-sm font-bold">{a.label}</p>
                <p className="mt-0.5 text-[11px] text-text-dim">
                  {a.description}
                </p>
                <p className="mt-1 text-[10px] uppercase tracking-widest text-text-dim">
                  {t("simulator.requires", "Requires")}: {a.required}
                </p>
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

function ToolPolicyCard({
  policy,
  t,
}: {
  policy: EffectiveToolPolicy | null;
  t: Translate;
}) {
  return (
    <Card padding="md">
      <SectionHeader
        icon={<Wrench className="h-4 w-4" />}
        title={t("simulator.tool_policy_title", "Tool policy (per-user)")}
        configured={!!policy}
      />
      {policy ? (
        <div className="grid gap-4 md:grid-cols-2">
          <div>
            <p className="text-[11px] uppercase tracking-widest text-success mb-1.5">
              {t("simulator.allowed_tools", "Allowed")}
            </p>
            <PatternList
              items={policy.allowed_tools}
              empty={t("simulator.no_allow_list", "No allow-list set")}
            />
          </div>
          <div>
            <p className="text-[11px] uppercase tracking-widest text-error mb-1.5">
              {t("simulator.denied_tools", "Denied")}
            </p>
            <PatternList
              items={policy.denied_tools}
              empty={t("simulator.no_deny_list", "No deny-list set")}
            />
          </div>
        </div>
      ) : (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.tool_policy_unset",
            "Defers to per-agent ToolPolicy and channel rules.",
          )}
        </p>
      )}
    </Card>
  );
}

function ToolCategoriesCard({
  categories,
  t,
}: {
  categories: EffectiveToolCategories | null;
  t: Translate;
}) {
  return (
    <Card padding="md">
      <SectionHeader
        icon={<Layers className="h-4 w-4" />}
        title={t(
          "simulator.tool_categories_title",
          "Tool categories (bulk by ToolGroup)",
        )}
        configured={!!categories}
      />
      {categories ? (
        <div className="grid gap-4 md:grid-cols-2">
          <div>
            <p className="text-[11px] uppercase tracking-widest text-success mb-1.5">
              {t("simulator.allowed_groups", "Allowed groups")}
            </p>
            <PatternList
              items={categories.allowed_groups}
              empty={t("simulator.no_allow_list", "No allow-list set")}
            />
          </div>
          <div>
            <p className="text-[11px] uppercase tracking-widest text-error mb-1.5">
              {t("simulator.denied_groups", "Denied groups")}
            </p>
            <PatternList
              items={categories.denied_groups}
              empty={t("simulator.no_deny_list", "No deny-list set")}
            />
          </div>
        </div>
      ) : (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.tool_categories_unset",
            "No category-level overrides for this user.",
          )}
        </p>
      )}
    </Card>
  );
}

function MemoryAccessCard({
  access,
  t,
}: {
  access: EffectiveMemoryAccess | null;
  t: Translate;
}) {
  return (
    <Card padding="md">
      <SectionHeader
        icon={<Database className="h-4 w-4" />}
        title={t("simulator.memory_title", "Memory access")}
        configured={!!access}
      />
      {access ? (
        <div className="space-y-3">
          <div className="flex flex-wrap gap-2">
            <Badge variant={access.pii_access ? "warning" : "default"}>
              {access.pii_access
                ? t("simulator.pii_access_on", "PII access ON")
                : t("simulator.pii_access_off", "PII redacted")}
            </Badge>
            <Badge variant={access.export_allowed ? "info" : "default"}>
              {access.export_allowed
                ? t("simulator.export_on", "Export allowed")
                : t("simulator.export_off", "No export")}
            </Badge>
            <Badge variant={access.delete_allowed ? "warning" : "default"}>
              {access.delete_allowed
                ? t("simulator.delete_on", "Delete allowed")
                : t("simulator.delete_off", "No delete")}
            </Badge>
          </div>
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <p className="text-[11px] uppercase tracking-widest text-text-dim mb-1.5">
                {t("simulator.readable_namespaces", "Readable namespaces")}
              </p>
              <PatternList
                items={access.readable_namespaces}
                empty={t("simulator.no_namespaces", "No namespaces")}
              />
            </div>
            <div>
              <p className="text-[11px] uppercase tracking-widest text-text-dim mb-1.5">
                {t("simulator.writable_namespaces", "Writable namespaces")}
              </p>
              <PatternList
                items={access.writable_namespaces}
                empty={t("simulator.no_namespaces", "No namespaces")}
              />
            </div>
          </div>
        </div>
      ) : (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.memory_unset",
            "Falls back to the role-default ACL (Owner/Admin = full, User = proactive + kv:*, Viewer = proactive read-only).",
          )}
        </p>
      )}
    </Card>
  );
}

function BudgetCard({
  budget,
  t,
}: {
  budget: EffectiveBudget | null;
  t: Translate;
}) {
  return (
    <Card padding="md">
      <SectionHeader
        icon={<DollarSign className="h-4 w-4" />}
        title={t("simulator.budget_title", "Per-user budget caps")}
        configured={!!budget}
      />
      {budget ? (
        <div className="grid gap-3 md:grid-cols-3">
          <BudgetRow
            label={t("simulator.budget_hourly", "Hourly")}
            value={budget.max_hourly_usd}
          />
          <BudgetRow
            label={t("simulator.budget_daily", "Daily")}
            value={budget.max_daily_usd}
          />
          <BudgetRow
            label={t("simulator.budget_monthly", "Monthly")}
            value={budget.max_monthly_usd}
          />
          <div className="md:col-span-3 text-[11px] text-text-dim">
            {t("simulator.alert_threshold_label", "Alert threshold")}:{" "}
            {(budget.alert_threshold * 100).toFixed(0)}%
          </div>
        </div>
      ) : (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.budget_unset",
            "No per-user cap. Bounded by global / per-agent / per-provider budgets only.",
          )}
        </p>
      )}
    </Card>
  );
}

function BudgetRow({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-xl border border-border-subtle p-3">
      <p className="text-[11px] uppercase tracking-widest text-text-dim">
        {label}
      </p>
      <p className="text-sm font-bold mt-1">
        {value > 0 ? `$${value.toFixed(2)}` : "—"}
      </p>
      {value === 0 ? (
        <p className="text-[10px] text-text-dim mt-0.5">unlimited on window</p>
      ) : null}
    </div>
  );
}

function ChannelRulesCard({
  rules,
  t,
}: {
  rules: Record<string, EffectiveChannelToolPolicy>;
  t: Translate;
}) {
  const entries = Object.entries(rules);
  return (
    <Card padding="md">
      <SectionHeader
        icon={<Radio className="h-4 w-4" />}
        title={t("simulator.channel_rules_title", "Per-channel tool overrides")}
        configured={entries.length > 0}
      />
      {entries.length === 0 ? (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.channel_rules_unset",
            "No per-channel overrides. The global ApprovalPolicy.channel_rules still applies.",
          )}
        </p>
      ) : (
        <div className="space-y-3">
          {entries.map(([channel, rule]) => (
            <div
              key={channel}
              className="rounded-xl border border-border-subtle p-3"
            >
              <p className="text-xs font-bold mb-2">{channel}</p>
              <div className="grid gap-3 md:grid-cols-2">
                <div>
                  <p className="text-[10px] uppercase tracking-widest text-success mb-1">
                    {t("simulator.allowed_tools", "Allowed")}
                  </p>
                  <PatternList
                    items={rule.allowed_tools}
                    empty={t("simulator.no_allow_list", "No allow-list set")}
                  />
                </div>
                <div>
                  <p className="text-[10px] uppercase tracking-widest text-error mb-1">
                    {t("simulator.denied_tools", "Denied")}
                  </p>
                  <PatternList
                    items={rule.denied_tools}
                    empty={t("simulator.no_deny_list", "No deny-list set")}
                  />
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

function ChannelBindingsCard({
  bindings,
  t,
}: {
  bindings: Record<string, string>;
  t: Translate;
}) {
  const entries = Object.entries(bindings);
  return (
    <Card padding="md">
      <SectionHeader
        icon={<Link2 className="h-4 w-4" />}
        title={t("simulator.channel_bindings_title", "Channel bindings")}
        configured={entries.length > 0}
      />
      {entries.length === 0 ? (
        <p className="text-xs text-text-dim italic">
          {t(
            "simulator.bindings_unset",
            "No platform IDs bound. Inbound from any channel will be unrecognised.",
          )}
        </p>
      ) : (
        <div className="space-y-1.5">
          {entries.map(([channel, platformId]) => (
            <div key={channel} className="flex items-center gap-2 text-xs">
              <Badge variant="info">{channel}</Badge>
              <code className="rounded-md border border-border-subtle bg-surface-2 px-1.5 py-0.5 text-[11px]">
                {platformId}
              </code>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
