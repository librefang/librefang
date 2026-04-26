// Permission simulator (RBAC M6).
//
// Pick a user → render the matrix of `Action` enum variants with the
// allow/deny decision derived locally from the kernel's role hierarchy.
// Lives entirely on the client so it stays useful even before the M3
// per-user-policy slice (#3205) ships its richer simulator endpoint.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Shield, CheckCircle2, XCircle } from "lucide-react";

import { useUsers } from "../lib/queries/users";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Select } from "../components/ui/Select";
import { EmptyState } from "../components/ui/EmptyState";

// Roles ordered weakest → strongest. Indices double as the comparator we
// use to derive allow/deny decisions, mirroring `UserRole as u8` in
// `librefang_kernel::auth::UserRole`.
const ROLE_ORDER = ["viewer", "user", "admin", "owner"] as const;
type Role = (typeof ROLE_ORDER)[number];

// Mirrors `librefang_kernel::auth::Action` + its `required_role` map. Keep
// in sync with the kernel: any change there must update both halves.
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

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        icon={<Shield className="h-4 w-4" />}
        title={t("simulator.title", "Permission simulator")}
        subtitle={t(
          "simulator.subtitle",
          "Pick a user and see which actions their role allows. Mirrors the kernel's UserRole hierarchy.",
        )}
        badge={t("simulator.badge", "Live")}
        helpText={t(
          "simulator.help",
          "The decision is computed locally from `UserRole` ordering (Viewer < User < Admin < Owner). When M3 (#3205) lands, per-user tool/memory policy will refine these results — the dashboard hook is already wired against /api/users/{name}/policy.",
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
      ) : selected ? (
        <Card padding="md">
          <div className="flex items-center gap-2 mb-4">
            <p className="text-sm font-bold">{selected.name}</p>
            <Badge variant="info">{selected.role}</Badge>
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
      ) : null}
    </div>
  );
}
