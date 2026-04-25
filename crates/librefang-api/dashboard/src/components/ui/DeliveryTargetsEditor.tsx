import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, X, AlertCircle, Send, Globe, FileText, Mail, MessageSquare } from "lucide-react";
import { Button } from "./Button";
import type { CronDeliveryTarget, CronDeliveryTargetType } from "../../lib/http/client";

// 业务说明: cron 多目标投递编辑器,UI 完全独立于具体页面,负责
// 列出现有 targets / 新增 / 删除。保存动作由父组件控制。
// Stripping empty optional fields keeps the payload aligned with the
// Rust `Option<String>` shape — sending "" would deserialize as Some("").

const INPUT_CLASS =
  "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";
const SMALL_LABEL = "text-[10px] font-bold text-text-dim uppercase";

const TYPE_OPTIONS: { value: CronDeliveryTargetType; labelKey: string; defaultLabel: string }[] = [
  { value: "channel", labelKey: "scheduler.delivery.type_channel", defaultLabel: "Channel" },
  { value: "webhook", labelKey: "scheduler.delivery.type_webhook", defaultLabel: "Webhook" },
  { value: "local_file", labelKey: "scheduler.delivery.type_local_file", defaultLabel: "Local file" },
  { value: "email", labelKey: "scheduler.delivery.type_email", defaultLabel: "Email" },
];

const CHANNEL_PRESETS: { value: string; label: string }[] = [
  { value: "telegram", label: "Telegram" },
  { value: "slack", label: "Slack" },
  { value: "discord", label: "Discord" },
  { value: "signal", label: "Signal" },
];

interface DeliveryTargetsEditorProps {
  /** Current targets (controlled). */
  value: CronDeliveryTarget[];
  /** Replace the full list. */
  onChange: (next: CronDeliveryTarget[]) => void;
  /** Disable all interactions (e.g. while a save mutation is pending). */
  disabled?: boolean;
}

export interface DraftState {
  type: CronDeliveryTargetType;
  channel_type: string;
  recipient: string;
  /** Slack thread_ts / Telegram forum-topic id; optional. */
  thread_id: string;
  /** Adapter-key suffix for multi-account adapters; optional. */
  account_id: string;
  url: string;
  auth_header: string;
  path: string;
  append: boolean;
  to: string;
  subject_template: string;
}

const EMPTY_DRAFT: DraftState = {
  type: "channel",
  channel_type: "telegram",
  recipient: "",
  thread_id: "",
  account_id: "",
  url: "",
  auth_header: "",
  path: "",
  append: true,
  to: "",
  subject_template: "",
};

/**
 * Build a `CronDeliveryTarget` payload from draft form state. Optional
 * empty-string fields are dropped so the JSON matches the Rust enum's
 * `#[serde(default)] Option<String>` shape exactly.
 *
 * Returns `[target | null, errorMessage | null]`. When the second slot is
 * non-null the draft is invalid; the caller should surface the message
 * and not commit.
 */
// Hostnames the backend SSRF-rejects. Mirrored here so users get
// instant feedback in the form instead of a round-trip + toast.
const SSRF_BLOCKED_HOSTS = new Set([
  "localhost",
  "metadata",
  "metadata.google.internal",
  "metadata.aws.amazon.com",
]);

function isBlockedWebhookHost(host: string): boolean {
  const lower = host.toLowerCase();
  if (SSRF_BLOCKED_HOSTS.has(lower)) return true;
  // Loopback IPv4 (127.0.0.0/8) and link-local (169.254.0.0/16, the
  // cloud-metadata range). String-only check — full CIDR parsing isn't
  // needed for an instant-feedback UX layer; the backend has the real
  // enforcement.
  if (/^127\./.test(host)) return true;
  if (/^169\.254\./.test(host)) return true;
  // IPv6 loopback / link-local.
  if (lower === "::1" || lower.startsWith("[::1]")) return true;
  if (lower.startsWith("fe80:") || lower.startsWith("[fe80:")) return true;
  return false;
}

/** Exported so unit tests can drive the validator directly without
 *  spinning up a full component render. Pure function — same input
 *  yields same `[target, error]` tuple, no side effects. */
export function buildTarget(d: DraftState): [CronDeliveryTarget | null, string | null] {
  if (d.type === "channel") {
    if (!d.channel_type.trim()) return [null, "scheduler.delivery.err_channel_type_required"];
    if (!d.recipient.trim()) return [null, "scheduler.delivery.err_recipient_required"];
    const target: CronDeliveryTarget = {
      type: "channel",
      channel_type: d.channel_type.trim(),
      recipient: d.recipient.trim(),
    };
    // 业务说明: 与 auth_header 一致 — 仅在用户填写时才下发,
    // 空字符串视为 `Option::None`,保持与 Rust 端 wire shape 对齐。
    if (d.thread_id.trim()) target.thread_id = d.thread_id.trim();
    if (d.account_id.trim()) target.account_id = d.account_id.trim();
    return [target, null];
  }
  if (d.type === "webhook") {
    const url = d.url.trim();
    if (!url) return [null, "scheduler.delivery.err_url_required"];
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      return [null, "scheduler.delivery.err_url_scheme"];
    }
    // Mirror the backend SSRF rejection (cron_delivery::validate_webhook_url)
    // so users see the error in the form, not after a save round-trip.
    try {
      const parsed = new URL(url);
      if (isBlockedWebhookHost(parsed.hostname)) {
        return [null, "scheduler.delivery.err_url_blocked_host"];
      }
    } catch {
      return [null, "scheduler.delivery.err_url_scheme"];
    }
    const t: CronDeliveryTarget = { type: "webhook", url };
    if (d.auth_header.trim()) t.auth_header = d.auth_header.trim();
    return [t, null];
  }
  if (d.type === "local_file") {
    const path = d.path.trim();
    if (!path) return [null, "scheduler.delivery.err_path_required"];
    // Mirror the backend `deliver_local_file` rejections so the form
    // surfaces the error before the save round-trip.
    if (path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path)) {
      return [null, "scheduler.delivery.err_path_absolute"];
    }
    if (path.split(/[\\/]/).some((seg) => seg === "..")) {
      return [null, "scheduler.delivery.err_path_traversal"];
    }
    return [{ type: "local_file", path, append: !!d.append }, null];
  }
  if (d.type === "email") {
    const to = d.to.trim();
    if (!to) return [null, "scheduler.delivery.err_email_required"];
    const t: CronDeliveryTarget = { type: "email", to };
    if (d.subject_template.trim()) t.subject_template = d.subject_template.trim();
    return [t, null];
  }
  return [null, "scheduler.delivery.err_pick_type"];
}

function targetIcon(t: CronDeliveryTarget): ReactNode {
  switch (t.type) {
    case "channel":
      return <MessageSquare className="w-3.5 h-3.5" />;
    case "webhook":
      return <Globe className="w-3.5 h-3.5" />;
    case "local_file":
      return <FileText className="w-3.5 h-3.5" />;
    case "email":
      return <Mail className="w-3.5 h-3.5" />;
  }
}

function targetSummary(t: CronDeliveryTarget): string {
  switch (t.type) {
    case "channel": {
      let s = `${t.channel_type} → ${t.recipient}`;
      // Surface the optional routing hints in the summary so the user
      // can see at a glance which target carries thread/account context.
      if (t.thread_id) s += ` · thread:${t.thread_id}`;
      if (t.account_id) s += ` · acct:${t.account_id}`;
      return s;
    }
    case "webhook":
      return t.url;
    case "local_file":
      return `${t.append ? "append" : "overwrite"} ${t.path}`;
    case "email":
      return t.subject_template ? `${t.to} · ${t.subject_template}` : t.to;
  }
}

function targetBadgeClass(t: CronDeliveryTarget): string {
  switch (t.type) {
    case "channel":
      return "bg-brand/10 text-brand";
    case "webhook":
      return "bg-success/10 text-success";
    case "local_file":
      return "bg-main text-text-dim";
    case "email":
      return "bg-warning/10 text-warning";
  }
}

export function DeliveryTargetsEditor({ value, onChange, disabled }: DeliveryTargetsEditorProps) {
  const { t } = useTranslation();
  const [showPicker, setShowPicker] = useState(false);
  const [draft, setDraft] = useState<DraftState>(EMPTY_DRAFT);
  const [error, setError] = useState<string | null>(null);

  const closeAndReset = () => {
    setShowPicker(false);
    setDraft(EMPTY_DRAFT);
    setError(null);
  };

  const handleAdd = () => {
    const [target, errKey] = buildTarget(draft);
    if (errKey || !target) {
      setError(errKey ?? "scheduler.delivery.err_pick_type");
      return;
    }
    onChange([...value, target]);
    closeAndReset();
  };

  const handleRemove = (idx: number) => {
    const next = value.slice();
    next.splice(idx, 1);
    onChange(next);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <span className={SMALL_LABEL}>
          {t("scheduler.delivery.targets_label", { defaultValue: "Delivery targets" })}
          <span className="ml-1 normal-case text-text-dim/40 font-normal">
            ({value.length})
          </span>
        </span>
        {!showPicker && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => setShowPicker(true)}
            disabled={disabled}
          >
            <Plus className="w-3.5 h-3.5" />
            {t("scheduler.delivery.add_target", { defaultValue: "Add target" })}
          </Button>
        )}
      </div>

      {/* Existing targets list */}
      {value.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border-subtle/60 px-3 py-3 text-[11px] text-text-dim/60">
          {t("scheduler.delivery.no_targets", {
            defaultValue:
              "No fan-out targets. The legacy single delivery still applies.",
          })}
        </div>
      ) : (
        <div className="space-y-1.5">
          {value.map((target, idx) => (
            <div
              key={idx}
              className="flex items-center gap-2 rounded-xl border border-border-subtle bg-surface px-3 py-2"
            >
              <span
                className={`shrink-0 inline-flex items-center gap-1 rounded-lg px-2 py-0.5 text-[10px] font-bold uppercase ${targetBadgeClass(target)}`}
              >
                {targetIcon(target)}
                {target.type}
              </span>
              <span className="flex-1 min-w-0 truncate text-xs font-mono text-text-main/80">
                {targetSummary(target)}
              </span>
              <button
                type="button"
                onClick={() => handleRemove(idx)}
                disabled={disabled}
                className="p-1 rounded-lg text-text-dim/40 hover:text-error hover:bg-error/10 transition-colors disabled:opacity-40"
                aria-label={t("common.remove", { defaultValue: "Remove" })}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Add-target picker */}
      {showPicker && (
        <div className="rounded-xl border border-brand/30 bg-brand/[0.03] p-3 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-bold uppercase text-brand/80">
              {t("scheduler.delivery.new_target", { defaultValue: "New target" })}
            </span>
            <button
              type="button"
              onClick={closeAndReset}
              className="p-1 rounded-lg text-text-dim/40 hover:text-text-main"
              aria-label={t("common.cancel")}
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>

          {/* Type tabs */}
          <div className="grid grid-cols-4 gap-1">
            {TYPE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                type="button"
                onClick={() => {
                  setDraft({ ...EMPTY_DRAFT, type: opt.value });
                  setError(null);
                }}
                className={`py-1.5 rounded-lg text-[10px] font-bold transition-colors ${
                  draft.type === opt.value
                    ? "bg-brand text-white"
                    : "bg-main text-text-dim hover:text-text-main"
                }`}
              >
                {t(opt.labelKey, { defaultValue: opt.defaultLabel })}
              </button>
            ))}
          </div>

          {/* Type-specific fields */}
          {draft.type === "channel" && (
            <div className="space-y-2">
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className={SMALL_LABEL}>
                    {t("scheduler.delivery.channel_type", { defaultValue: "Channel type" })}
                  </label>
                  <select
                    value={
                      CHANNEL_PRESETS.some((p) => p.value === draft.channel_type)
                        ? draft.channel_type
                        : "__custom__"
                    }
                    onChange={(e) => {
                      const v = e.target.value;
                      setDraft({
                        ...draft,
                        channel_type: v === "__custom__" ? "" : v,
                      });
                    }}
                    className={INPUT_CLASS}
                  >
                    {CHANNEL_PRESETS.map((p) => (
                      <option key={p.value} value={p.value}>
                        {p.label}
                      </option>
                    ))}
                    <option value="__custom__">
                      {t("scheduler.delivery.custom", { defaultValue: "Custom…" })}
                    </option>
                  </select>
                  {!CHANNEL_PRESETS.some((p) => p.value === draft.channel_type) && (
                    <input
                      value={draft.channel_type}
                      onChange={(e) => setDraft({ ...draft, channel_type: e.target.value })}
                      placeholder="e.g. mattermost"
                      className={`${INPUT_CLASS} mt-1 font-mono text-xs`}
                    />
                  )}
                </div>
                <div>
                  <label className={SMALL_LABEL}>
                    {t("scheduler.delivery.recipient", { defaultValue: "Recipient" })}
                  </label>
                  <input
                    value={draft.recipient}
                    onChange={(e) => setDraft({ ...draft, recipient: e.target.value })}
                    placeholder="chat ID / channel ID"
                    className={`${INPUT_CLASS} font-mono text-xs`}
                  />
                </div>
              </div>
              {/*
                Helper text instead of a hard whitelist on `channel_type`:
                forward-compat with adapters not yet shipped on the
                client. Bad values still fail loudly at delivery time
                (the kernel surfaces the available adapter list in the
                error message).
              */}
              <p className="text-[10px] text-text-dim/60 leading-snug">
                {t("scheduler.delivery.channel_type_helper", {
                  defaultValue:
                    "Custom channel types are passed through as-is. Unknown values fail at delivery time with the list of configured adapters.",
                })}
              </p>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className={SMALL_LABEL}>
                    {t("scheduler.delivery.thread_id", {
                      defaultValue: "Thread ID (optional)",
                    })}
                  </label>
                  <input
                    value={draft.thread_id}
                    onChange={(e) => setDraft({ ...draft, thread_id: e.target.value })}
                    placeholder="Slack thread_ts / Telegram topic"
                    className={`${INPUT_CLASS} font-mono text-xs`}
                  />
                </div>
                <div>
                  <label className={SMALL_LABEL}>
                    {t("scheduler.delivery.account_id", {
                      defaultValue: "Account ID (optional)",
                    })}
                  </label>
                  <input
                    value={draft.account_id}
                    onChange={(e) => setDraft({ ...draft, account_id: e.target.value })}
                    placeholder="workspace-b"
                    className={`${INPUT_CLASS} font-mono text-xs`}
                  />
                </div>
              </div>
            </div>
          )}

          {draft.type === "webhook" && (
            <div className="space-y-2">
              <div>
                <label className={SMALL_LABEL}>
                  {t("scheduler.delivery.url", { defaultValue: "Webhook URL" })}
                </label>
                <input
                  value={draft.url}
                  onChange={(e) => setDraft({ ...draft, url: e.target.value })}
                  placeholder="https://example.com/hook"
                  className={`${INPUT_CLASS} font-mono text-xs`}
                />
              </div>
              <div>
                <label className={SMALL_LABEL}>
                  {t("scheduler.delivery.auth_header", {
                    defaultValue: "Authorization header (optional)",
                  })}
                </label>
                <input
                  value={draft.auth_header}
                  onChange={(e) => setDraft({ ...draft, auth_header: e.target.value })}
                  placeholder="Bearer ..."
                  className={`${INPUT_CLASS} font-mono text-xs`}
                />
              </div>
            </div>
          )}

          {draft.type === "local_file" && (
            <div className="space-y-2">
              <div>
                <label className={SMALL_LABEL}>
                  {t("scheduler.delivery.path", { defaultValue: "File path" })}
                </label>
                <input
                  value={draft.path}
                  onChange={(e) => setDraft({ ...draft, path: e.target.value })}
                  placeholder="/var/log/cron-output.log"
                  className={`${INPUT_CLASS} font-mono text-xs`}
                />
              </div>
              <label className="flex items-center gap-2 text-xs text-text-dim cursor-pointer">
                <input
                  type="checkbox"
                  checked={draft.append}
                  onChange={(e) => setDraft({ ...draft, append: e.target.checked })}
                  className="rounded border-border-subtle"
                />
                {t("scheduler.delivery.append", {
                  defaultValue: "Append (uncheck to overwrite each run)",
                })}
              </label>
            </div>
          )}

          {draft.type === "email" && (
            <div className="space-y-2">
              <div>
                <label className={SMALL_LABEL}>
                  {t("scheduler.delivery.email_to", { defaultValue: "Recipient email" })}
                </label>
                <input
                  type="email"
                  value={draft.to}
                  onChange={(e) => setDraft({ ...draft, to: e.target.value })}
                  placeholder="alerts@example.com"
                  className={`${INPUT_CLASS} font-mono text-xs`}
                />
              </div>
              <div>
                <label className={SMALL_LABEL}>
                  {t("scheduler.delivery.subject_template", {
                    defaultValue: "Subject template (optional, {job} = job name)",
                  })}
                </label>
                <input
                  value={draft.subject_template}
                  onChange={(e) => setDraft({ ...draft, subject_template: e.target.value })}
                  placeholder="Cron: {job}"
                  className={`${INPUT_CLASS} font-mono text-xs`}
                />
              </div>
            </div>
          )}

          {error && (
            <div className="flex items-center gap-1.5 text-error text-[11px]">
              <AlertCircle className="w-3.5 h-3.5 shrink-0" />
              <span>{t(error, { defaultValue: error })}</span>
            </div>
          )}

          <div className="flex gap-2 pt-1">
            <Button
              type="button"
              variant="primary"
              size="sm"
              className="flex-1"
              onClick={handleAdd}
              disabled={disabled}
            >
              <Send className="w-3.5 h-3.5" />
              {t("scheduler.delivery.add", { defaultValue: "Add target" })}
            </Button>
            <Button type="button" variant="secondary" size="sm" onClick={closeAndReset}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
