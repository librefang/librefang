import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, ShieldCheck, ShieldOff, AlertTriangle } from "lucide-react";
import {
  type McpServerConfigured,
  type McpTaintPolicy,
  type McpTaintToolPolicy,
  type McpTaintToolAction,
  type McpTaintPathPolicy,
  type TaintRuleId,
} from "../api";
import { useUpdateMcpTaintPolicy } from "../lib/mutations/mcp";
import { Modal } from "./ui/Modal";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { useUIStore } from "../lib/store";

const RULE_IDS: TaintRuleId[] = [
  "authorization_literal",
  "key_value_secret",
  "well_known_prefix",
  "opaque_token",
  "pii_email",
  "pii_phone",
  "pii_credit_card",
  "pii_ssn",
  "sensitive_key_name",
];

/** Short, human-readable hint shown next to each rule ID. */
const RULE_LABELS: Record<TaintRuleId, string> = {
  authorization_literal: "Blocks `Authorization:` header prefix",
  key_value_secret: "Blocks `key=value` / `key: value` secret shapes",
  well_known_prefix: "Blocks `sk-` / `ghp_` / `AKIA` / `AIza` prefixes",
  opaque_token: "Blocks long mixed-alnum opaque tokens",
  pii_email: "Blocks e-mail addresses",
  pii_phone: "Blocks phone numbers",
  pii_credit_card: "Blocks credit-card numbers",
  pii_ssn: "Blocks Social Security numbers",
  sensitive_key_name: "Blocks JSON keys named `authorization`/`api_key`/...",
};

type DraftPaths = Record<string, McpTaintPathPolicy>;
type DraftTools = Record<string, McpTaintToolPolicy>;

/**
 * Issue #3050: granular taint-policy tree editor.
 *
 * Renders a server → tool → path tree editable in place. Saves through the
 * dedicated PATCH `/api/mcp/servers/{id}/taint` endpoint via
 * `useUpdateMcpTaintPolicy`, which invalidates the matching query keys so
 * the underlying card refreshes without a full config reload. The PATCH
 * endpoint is preferred over the generic PUT so unrelated `McpServerConfig`
 * fields aren't revalidated on every taint edit.
 *
 * NOTE on rule_set overlap: when a tool references multiple `[[taint_rules]]`
 * sets that all cover the same rule, the *most permissive* action wins
 * (`log` > `warn` > `block`). Adding an `audit_only` rule set with
 * `action = "log"` will silently neutralise any `block` set that overlaps
 * on the same rule — this is by design, but counter-intuitive. See the hint
 * shown next to the `rule_sets` field.
 */
export function TaintPolicyEditor({
  server,
  isOpen,
  onClose,
}: {
  server: McpServerConfigured;
  isOpen: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const mutation = useUpdateMcpTaintPolicy();

  const [scanning, setScanning] = useState<boolean>(server.taint_scanning ?? true);
  const [tools, setTools] = useState<DraftTools>(
    () => deepClone(server.taint_policy?.tools ?? {}),
  );

  // ── tool-level helpers ────────────────────────────────────────────────
  const setToolField = useCallback(
    <K extends keyof McpTaintToolPolicy>(name: string, key: K, value: McpTaintToolPolicy[K]) => {
      setTools((prev) => ({
        ...prev,
        [name]: { ...(prev[name] ?? {}), [key]: value } as McpTaintToolPolicy,
      }));
    },
    [],
  );

  const addTool = useCallback(() => {
    const fresh = uniqueToolName(tools, "tool_name");
    setTools((prev) => ({ ...prev, [fresh]: { default: "scan", paths: {}, rule_sets: [] } }));
  }, [tools]);

  const removeTool = useCallback((name: string) => {
    setTools((prev) => {
      const next = { ...prev };
      delete next[name];
      return next;
    });
  }, []);

  const renameTool = useCallback((oldName: string, newName: string) => {
    if (!newName.trim() || newName === oldName) return;
    setTools((prev) => {
      if (newName in prev) return prev;
      const next: DraftTools = {};
      for (const [k, v] of Object.entries(prev)) {
        next[k === oldName ? newName : k] = v;
      }
      return next;
    });
  }, []);

  // ── path-level helpers ────────────────────────────────────────────────
  const setPaths = useCallback((toolName: string, paths: DraftPaths) => {
    setTools((prev) => ({ ...prev, [toolName]: { ...(prev[toolName] ?? {}), paths } }));
  }, []);

  const handleSave = useCallback(() => {
    const cleaned = cleanTools(tools);
    const policy: McpTaintPolicy | undefined = Object.keys(cleaned).length
      ? { tools: cleaned }
      : undefined;
    const id =
      (server as unknown as { id?: string; name?: string }).id ??
      (server as unknown as { name: string }).name;
    mutation.mutate(
      {
        id,
        taint_scanning: scanning,
        taint_policy: policy,
      },
      {
        onSuccess: () => {
          addToast(t("mcp.taint_policy_saved", "Taint policy saved"), "success");
          onClose();
        },
        onError: (e: unknown) =>
          addToast(
            e instanceof Error ? e.message : t("mcp.taint_policy_save_failed", "Failed to save taint policy"),
            "error",
          ),
      },
    );
  }, [tools, scanning, mutation, server, addToast, onClose, t]);

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={t("mcp.taint_policy_title", "Taint policy — {{name}}", { name: server.name })}
      size="3xl"
      variant="panel-right"
    >
      <div className="flex flex-col gap-4 p-4 overflow-y-auto">
        {/* Server-level scanning toggle */}
        <section className="rounded-xl border border-border-subtle p-4 bg-main/40">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-start gap-3">
              {scanning ? (
                <ShieldCheck className="h-5 w-5 text-success mt-0.5" />
              ) : (
                <ShieldOff className="h-5 w-5 text-warning mt-0.5" />
              )}
              <div>
                <p className="font-bold text-sm">
                  {t("mcp.taint_scanning_label", "Outbound taint scanning")}
                </p>
                <p className="text-xs text-text-dim">
                  {t(
                    "mcp.taint_scanning_help",
                    "When off, no content heuristic runs on this server's tool arguments. Key-name blocking and the per-tool rules below still apply when on.",
                  )}
                </p>
              </div>
            </div>
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={scanning}
                onChange={(e) => setScanning(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-border-subtle peer-checked:bg-success rounded-full transition-colors" />
              <div className="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform peer-checked:translate-x-5" />
            </label>
          </div>
        </section>

        {!scanning && (
          <div className="rounded-xl border border-warning/30 bg-warning/5 p-3 flex items-start gap-2">
            <AlertTriangle className="h-4 w-4 text-warning mt-0.5 shrink-0" />
            <p className="text-xs text-text-dim">
              {t(
                "mcp.taint_scanning_off_warning",
                "Per-tool exemptions below are ignored while server-level scanning is off. Re-enable scanning to use the granular policy.",
              )}
            </p>
          </div>
        )}

        {/* Tool tree */}
        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-bold">
              {t("mcp.taint_tools_heading", "Per-tool exemptions")}
            </h3>
            <Button size="sm" variant="secondary" onClick={addTool}>
              <Plus className="h-3.5 w-3.5" />
              {t("mcp.taint_add_tool", "Add tool policy")}
            </Button>
          </div>

          {Object.keys(tools).length === 0 ? (
            <p className="text-xs text-text-dim italic">
              {t(
                "mcp.taint_no_tool_policies",
                "No per-tool exemptions yet. Add one to skip individual rules for known-safe arguments.",
              )}
            </p>
          ) : (
            Object.entries(tools).map(([name, policy]) => (
              <ToolPolicyRow
                key={name}
                name={name}
                policy={policy}
                onRename={(newName) => renameTool(name, newName)}
                onRemove={() => removeTool(name)}
                onChangeDefault={(action) => setToolField(name, "default", action)}
                onChangeRuleSets={(sets) => setToolField(name, "rule_sets", sets)}
                onChangePaths={(paths) => setPaths(name, paths)}
              />
            ))
          )}
        </section>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 pt-2 border-t border-border-subtle mt-2">
          <Button variant="ghost" onClick={onClose} disabled={mutation.isPending}>
            {t("common.cancel")}
          </Button>
          <Button onClick={handleSave} disabled={mutation.isPending}>
            {mutation.isPending ? t("common.saving") : t("common.save")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ── Sub-components ───────────────────────────────────────────────────────

function ToolPolicyRow({
  name,
  policy,
  onRename,
  onRemove,
  onChangeDefault,
  onChangeRuleSets,
  onChangePaths,
}: {
  name: string;
  policy: McpTaintToolPolicy;
  onRename: (newName: string) => void;
  onRemove: () => void;
  onChangeDefault: (action: McpTaintToolAction) => void;
  onChangeRuleSets: (sets: string[]) => void;
  onChangePaths: (paths: DraftPaths) => void;
}) {
  const { t } = useTranslation();
  const [localName, setLocalName] = useState(name);
  const [ruleSetsText, setRuleSetsText] = useState((policy.rule_sets ?? []).join(", "));
  const paths = policy.paths ?? {};

  const addPath = useCallback(() => {
    const newKey = uniquePathKey(paths, "$.field");
    onChangePaths({ ...paths, [newKey]: { skip_rules: [] } });
  }, [paths, onChangePaths]);

  const removePath = useCallback(
    (key: string) => {
      const next = { ...paths };
      delete next[key];
      onChangePaths(next);
    },
    [paths, onChangePaths],
  );

  const renamePath = useCallback(
    (oldKey: string, newKey: string) => {
      if (!newKey.trim() || newKey === oldKey) return;
      if (newKey in paths) return;
      const next: DraftPaths = {};
      for (const [k, v] of Object.entries(paths)) {
        next[k === oldKey ? newKey : k] = v;
      }
      onChangePaths(next);
    },
    [paths, onChangePaths],
  );

  const setPathSkipRules = useCallback(
    (key: string, skipRules: TaintRuleId[]) => {
      onChangePaths({ ...paths, [key]: { skip_rules: skipRules } });
    },
    [paths, onChangePaths],
  );

  return (
    <div className="rounded-xl border border-border-subtle bg-main/40 p-3 space-y-3">
      {/* Tool name row */}
      <div className="flex items-center gap-2">
        <Badge variant="default">{t("mcp.taint_tool", "tool")}</Badge>
        <Input
          value={localName}
          onChange={(e) => setLocalName(e.target.value)}
          onBlur={() => onRename(localName.trim())}
          placeholder="tool_name"
          className="font-mono text-xs"
        />
        <Select
          value={policy.default ?? "scan"}
          onChange={(e) => onChangeDefault(e.target.value as McpTaintToolAction)}
          options={[
            { value: "scan", label: "scan (default)" },
            { value: "skip", label: "skip (bypass scanning)" },
          ]}
          className="text-xs"
        />
        <button
          onClick={onRemove}
          className="p-1.5 rounded-lg text-text-dim hover:text-error hover:bg-error/10 transition-colors"
          aria-label={t("mcp.taint_remove_tool", "Remove tool policy")}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Rule sets reference */}
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-xs">
          <span className="text-text-dim font-bold w-20">rule_sets</span>
          <Input
            value={ruleSetsText}
            onChange={(e) => setRuleSetsText(e.target.value)}
            onBlur={() =>
              onChangeRuleSets(
                ruleSetsText
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean),
              )
            }
            placeholder={t(
              "mcp.taint_rule_sets_placeholder",
              "comma-separated names from [[taint_rules]]",
            )}
            className="text-xs font-mono"
            disabled={policy.default === "skip"}
          />
        </div>
        {ruleSetsText.trim().length > 0 && (
          <p className="text-[11px] italic text-text-dim pl-22">
            {t(
              "mcp.taint_rule_sets_overlap_hint",
              "When sets overlap on the same rule, the most permissive action wins (log > warn > block) — an audit-only set will silently neutralise a block set on the shared rule.",
            )}
          </p>
        )}
        {policy.default === "skip" && (policy.rule_sets ?? []).length > 0 ? (
          <p className="text-[11px] italic text-warning pl-22">
            {t(
              "mcp.taint_skip_rule_sets_hint",
              "default = skip bypasses scanning entirely — rule_sets above will not fire (not even Log). Switch to default = scan for audit-only rule sets.",
            )}
          </p>
        ) : null}
      </div>

      {/* Paths */}
      {policy.default === "skip" ? (
        <p className="text-[11px] italic text-text-dim">
          {t(
            "mcp.taint_default_skip_hint",
            "default = skip — argument paths below are ignored because scanning is bypassed for this tool.",
          )}
        </p>
      ) : (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-text-dim font-bold">
              {t("mcp.taint_paths_heading", "Path exemptions")}
            </span>
            <button
              onClick={addPath}
              className="text-[11px] font-bold text-brand hover:text-brand/80 transition-colors flex items-center gap-1"
            >
              <Plus className="h-3 w-3" />
              {t("mcp.taint_add_path", "Add path")}
            </button>
          </div>
          {Object.entries(paths).length === 0 ? (
            <p className="text-[11px] italic text-text-dim">
              {t(
                "mcp.taint_no_paths",
                "No path exemptions. The full rule set applies to every argument.",
              )}
            </p>
          ) : (
            <ul className="space-y-2">
              {Object.entries(paths).map(([key, p]) => (
                <PathRow
                  key={key}
                  pathKey={key}
                  policy={p}
                  onRename={(newKey) => renamePath(key, newKey)}
                  onChangeSkipRules={(rules) => setPathSkipRules(key, rules)}
                  onRemove={() => removePath(key)}
                />
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

function PathRow({
  pathKey,
  policy,
  onRename,
  onChangeSkipRules,
  onRemove,
}: {
  pathKey: string;
  policy: McpTaintPathPolicy;
  onRename: (newKey: string) => void;
  onChangeSkipRules: (rules: TaintRuleId[]) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(pathKey);
  const skipSet = useMemo(() => new Set(policy.skip_rules), [policy.skip_rules]);

  const toggle = (rule: TaintRuleId) => {
    const next = new Set(skipSet);
    if (next.has(rule)) next.delete(rule);
    else next.add(rule);
    onChangeSkipRules(Array.from(next));
  };

  return (
    <li className="rounded-lg border border-border-subtle/60 bg-surface-base p-2.5 space-y-2">
      <div className="flex items-center gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => onRename(draft.trim())}
          placeholder="$.path.to.field"
          className="font-mono text-xs"
        />
        <button
          onClick={onRemove}
          className="p-1 rounded text-text-dim hover:text-error hover:bg-error/10 transition-colors"
          aria-label={t("mcp.taint_remove_path", "Remove path exemption")}
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {RULE_IDS.map((rule) => {
          const active = skipSet.has(rule);
          return (
            <button
              key={rule}
              onClick={() => toggle(rule)}
              title={RULE_LABELS[rule]}
              className={`text-[10px] font-mono px-2 py-1 rounded-md border transition-colors ${
                active
                  ? "bg-brand/10 border-brand/40 text-brand"
                  : "bg-main/40 border-border-subtle text-text-dim hover:text-text-main"
              }`}
            >
              {rule}
            </button>
          );
        })}
      </div>
    </li>
  );
}

// ── Helpers ──────────────────────────────────────────────────────────────

function deepClone<T>(v: T): T {
  return typeof structuredClone === "function"
    ? structuredClone(v)
    : (JSON.parse(JSON.stringify(v)) as T);
}

function uniqueToolName(tools: DraftTools, base: string): string {
  if (!(base in tools)) return base;
  let i = 1;
  while (`${base}_${i}` in tools) i++;
  return `${base}_${i}`;
}

function uniquePathKey(paths: DraftPaths, base: string): string {
  if (!(base in paths)) return base;
  let i = 1;
  while (`${base}_${i}` in paths) i++;
  return `${base}_${i}`;
}

/** Drop empty inner shapes so the saved policy stays compact. */
function cleanTools(tools: DraftTools): DraftTools {
  const out: DraftTools = {};
  for (const [name, policy] of Object.entries(tools)) {
    if (!name.trim()) continue;
    const cleanPaths: DraftPaths = {};
    for (const [k, v] of Object.entries(policy.paths ?? {})) {
      if (k.trim() && (v.skip_rules ?? []).length > 0) cleanPaths[k] = v;
    }
    const ruleSets = (policy.rule_sets ?? []).filter((s) => s.trim().length > 0);
    const isEmpty =
      (policy.default ?? "scan") === "scan" &&
      Object.keys(cleanPaths).length === 0 &&
      ruleSets.length === 0;
    if (isEmpty) continue;
    out[name] = {
      default: policy.default ?? "scan",
      paths: cleanPaths,
      rule_sets: ruleSets,
    };
  }
  return out;
}
