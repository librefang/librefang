import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  wechatQrStart,
  wechatQrStatus,
  whatsappQrStart,
  whatsappQrStatus,
  type ChannelField,
  type ChannelInstance,
  type ChannelItem,
} from "../api";
import { useChannels, useChannelInstances } from "../lib/queries/channels";
import {
  useConfigureChannel,
  useCreateChannelInstance,
  useDeleteChannelInstance,
  useReloadChannels,
  useTestChannel,
  useUpdateChannelInstance,
} from "../lib/mutations/channels";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { copyToClipboard } from "../lib/clipboard";
import QRCode from "qrcode";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import {
  Network, Search, CheckCircle2, XCircle, ChevronRight, X, Grid3X3, List,
  Settings, AlertCircle, CheckSquare, Square, Plus, Trash2, Pencil, ArrowLeft,
  MessageCircle, Mail, Phone, Link2, Radio, Send, Bell, Wifi, Globe
} from "lucide-react";

const channelIcons: Record<string, React.ReactNode> = {
  slack: <MessageCircle className="w-5 h-5" />,
  discord: <MessageCircle className="w-5 h-5" />,
  telegram: <Send className="w-5 h-5" />,
  whatsapp: <Phone className="w-5 h-5" />,
  email: <Mail className="w-5 h-5" />,
  sms: <MessageCircle className="w-5 h-5" />,
  webhook: <Link2 className="w-5 h-5" />,
  http: <Globe className="w-5 h-5" />,
  websocket: <Radio className="w-5 h-5" />,
  mqtt: <Wifi className="w-5 h-5" />,
  slack_events: <Bell className="w-5 h-5" />,
  teams: <MessageCircle className="w-5 h-5" />,
};

function getChannelIcon(name: string): React.ReactNode {
  const key = name.toLowerCase().split("-")[0];
  return channelIcons[key] || <Radio className="w-5 h-5" />;
}

type SortField = "name" | "category";
type SortOrder = "asc" | "desc";
type ViewMode = "grid" | "list";

type Channel = ChannelItem;

interface ChannelCardProps {
  channel: Channel;
  isSelected: boolean;
  viewMode: ViewMode;
  onSelect: (name: string, checked: boolean) => void;
  onConfigure: (channel: Channel) => void;
  onViewDetails: (channel: Channel) => void;
  t: (key: string, opts?: { defaultValue?: string }) => string;
}

const ChannelCard = memo(function ChannelCard({ channel: c, isSelected, viewMode, onSelect, onConfigure, onViewDetails, t }: ChannelCardProps) {
  // Whole-card click opens the details drawer. Inner controls
  // (checkbox, Configure button) call e.stopPropagation() so the
  // card-level handler doesn't fire when the user clicks them.
  // Keyboard: Enter / Space on the focused card mirrors the click —
  // `role="button" + tabIndex={0}` makes the card itself focusable.
  // The trailing chevron is now decorative (`aria-hidden`) since the
  // entire surface is the activator.
  const openDetails = () => onViewDetails(c);
  const cardKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openDetails();
    }
  };
  const cardA11y = {
    onClick: openDetails,
    onKeyDown: cardKeyDown,
    role: "button" as const,
    tabIndex: 0,
    "aria-label": c.display_name || c.name,
  };

  // Compact card matching the design canvas: 30×30 accent icon, mono
  // name, mono `kind · N msgs/24h` sub-line, status dot. Both list and
  // grid views use the same shape now since the page only shows
  // configured channels (configure-flow chips moved to the picker
  // drawer where they actually help selection).
  const msgs = typeof c.msgs_24h === "number" ? c.msgs_24h : 0;
  const kind = c.category || c.name;
  // #4837: when a channel type has multiple instances configured, surface
  // the count as a left-side meta tag so cards make sense at a glance.
  const instanceCount = typeof c.instance_count === "number" ? c.instance_count : (c.configured ? 1 : 0);
  return (
    <Card
      hover
      padding="sm"
      className={`flex items-center gap-3 group transition-all focus-visible:ring-2 focus-visible:ring-brand/40 focus-visible:outline-none ${isSelected ? "ring-2 ring-brand" : ""}`}
      {...cardA11y}
    >
      <button
        onClick={(e) => { e.stopPropagation(); onSelect(c.name, !isSelected); }}
        className="shrink-0 text-text-dim hover:text-brand transition-colors"
        aria-label={isSelected ? t("common.deselect", { defaultValue: "Deselect" }) : t("common.select", { defaultValue: "Select" })}
      >
        {isSelected ? <CheckSquare className="w-4 h-4 text-brand" /> : <Square className="w-4 h-4" />}
      </button>
      <div className="w-[30px] h-[30px] rounded-[7px] bg-accent/10 border border-accent/30 text-accent grid place-items-center shrink-0">
        {getChannelIcon(c.name)}
      </div>
      <div className="min-w-0 flex-1">
        <div className="font-mono text-[13px] truncate text-text-main flex items-center gap-1.5">
          {c.display_name || c.name}
          {instanceCount > 1 && (
            <span className="font-mono text-[10px] px-1.5 py-0.5 rounded bg-brand/10 text-brand border border-brand/20">
              {t("channels.instance_count_short", { defaultValue: `${instanceCount}×` })}
            </span>
          )}
        </div>
        <div className="font-mono text-[11px] text-text-dim mt-0.5 truncate">
          {kind} · {msgs} {t("channels.msgs_24h", { defaultValue: "msgs/24h" })}
        </div>
      </div>
      {/* Status dot — running when there's recent activity, idle otherwise.
          Matches the design's `status: 'running' | 'idle'` field. */}
      <Badge variant={msgs > 0 ? "success" : "default"} dot className="shrink-0">
        <span className="sr-only">
          {msgs > 0 ? t("common.running") : t("common.idle")}
        </span>
      </Badge>
      <button
        type="button"
        onClick={(e) => { e.stopPropagation(); onConfigure(c); }}
        className="shrink-0 p-1.5 rounded-md text-text-dim hover:text-text-main hover:bg-main/40 transition-colors"
        aria-label={t("channels.config")}
        title={t("channels.config")}
      >
        <Settings className="w-3.5 h-3.5" />
      </button>
      {viewMode === "grid" && (
        <ChevronRight className="w-4 h-4 text-text-dim/60 shrink-0" aria-hidden="true" />
      )}
    </Card>
  );
});

// Details Modal
function DetailsModal({ channel, onClose, onConfigure, onTest, t }: {
  channel: Channel;
  onClose: () => void;
  onConfigure: () => void;
  onTest: () => void;
  t: (key: string) => string
}) {
  return (
    <DrawerPanel isOpen onClose={onClose} size="lg" hideCloseButton>
        {/* Coloured strip + custom header are kept inline so the
            configured/unconfigured stripe still renders. */}
        <div className={`h-2 bg-linear-to-r ${channel.configured ? "from-success via-success/60 to-success/30" : "from-brand via-brand/60 to-brand/30"}`} />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${channel.configured ? "bg-success/10 border border-success/20" : "bg-brand/10 border border-brand/20"}`}>
                {getChannelIcon(channel.name)}
              </div>
              <div>
                <h2 className="text-xl font-black">{channel.display_name || channel.name}</h2>
                <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">{channel.category || channel.name}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors" aria-label={t("common.close")}>
              <X className="w-5 h-5 text-text-dim" />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="p-6 space-y-4">
          <div className="p-4 rounded-xl bg-main/30">
            <p className="text-xs text-text-dim italic">{channel.description || "-"}</p>
          </div>

          <div className="space-y-3">
            <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("common.properties")}</h3>
            <div className="space-y-2">
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("common.status")}</span>
                <Badge variant={channel.configured ? "success" : "warning"}>
                  {channel.configured ? t("common.online") : t("common.setup")}
                </Badge>
              </div>
              {channel.difficulty && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.difficulty")}</span>
                  <span className="text-xs font-bold">{channel.difficulty}</span>
                </div>
              )}
              {channel.setup_time && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.setup_time")}</span>
                  <span className="text-xs font-bold">{channel.setup_time}</span>
                </div>
              )}
              {channel.setup_type && (
                <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                  <span className="text-xs font-bold text-text-dim">{t("channels.setup_type")}</span>
                  <span className="text-xs font-bold">{channel.setup_type}</span>
                </div>
              )}
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("channels.has_token")}</span>
                <span className={`text-xs font-bold ${channel.has_token ? "text-success" : "text-warning"}`}>
                  {channel.has_token ? t("common.yes") : t("common.no")}
                </span>
              </div>
            </div>
          </div>

          {/* Webhook Endpoint */}
          {channel.webhook_endpoint && (
            <div className="space-y-2">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">Webhook Endpoint</h3>
              <div className="p-3 rounded-lg bg-brand/5 border border-brand/20">
                <code className="text-xs font-mono text-brand break-all select-all">{channel.webhook_endpoint}</code>
                <p className="text-[10px] text-text-dim mt-1">Configure this path on the external platform. Port is the API listen port (default 4545).</p>
              </div>
            </div>
          )}

          {/* Setup Steps */}
          {channel.setup_steps && channel.setup_steps.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("channels.setup_steps")}</h3>
              <div className="space-y-2">
                {channel.setup_steps.map((step, idx) => (
                  <div key={idx} className="flex items-start gap-3 p-3 rounded-lg bg-main/20">
                    <span className="w-5 h-5 rounded-full bg-brand/20 text-brand text-xs font-bold flex items-center justify-center shrink-0">{idx + 1}</span>
                    <p className="text-xs text-text-main">{step}</p>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Fields */}
          {channel.fields && channel.fields.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-black uppercase tracking-wider text-text-dim">{t("channels.required_fields")}</h3>
              <div className="space-y-2">
                {channel.fields.map((field, idx) => (
                  <div key={idx} className="flex items-center justify-between p-3 rounded-lg bg-main/20">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-bold text-text-main">{field.label || field.key}</span>
                      {field.required && <span className="text-error text-[10px]">*</span>}
                    </div>
                    <div className="flex items-center gap-2">
                      {field.has_value ? (
                        <CheckCircle2 className="w-4 h-4 text-success" />
                      ) : (
                        <AlertCircle className="w-4 h-4 text-warning" />
                      )}
                      {field.env_var && (
                        <span className="text-[10px] font-mono text-text-dim">{field.env_var}</span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Actions */}
          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1" onClick={onConfigure} leftIcon={<Settings className="w-4 h-4" />}>
              {channel.configured ? t("channels.update_config") : t("channels.setup_adapter")}
            </Button>
            {channel.configured && (
              <Button variant="secondary" onClick={onTest} leftIcon={<CheckCircle2 className="w-4 h-4" />}>
                {t("channels.test") || "Test"}
              </Button>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
    </DrawerPanel>
  );
}

// Form for one channel instance — used by both the create-new and edit-existing
// flows in `InstancesDialog`. The same form was previously the whole of
// `ConfigDialog` and drove the legacy single-instance `/configure` endpoint.
//
// `fields` is the schema (with `value` / `has_value` populated for edits).
// `onSubmit` receives the non-empty, non-readonly values; the parent decides
// which mutation to fire.
function ChannelForm({
  channel,
  fields,
  description,
  submitLabel,
  isPending,
  onSubmit,
  onCancel,
  t,
}: {
  channel: Channel;
  fields: ChannelField[];
  description?: string;
  submitLabel: string;
  isPending: boolean;
  onSubmit: (payload: Record<string, string>) => void;
  onCancel: () => void;
  t: (key: string, opts?: { defaultValue?: string }) => string;
}) {
  const addToast = useUIStore((s) => s.addToast);
  const visibleSchema = useMemo(() => fields.filter(f => !f.advanced), [fields]);

  const initialValues = useMemo(() => {
    const vals: Record<string, string> = {};
    for (const f of visibleSchema) {
      if (f.readonly) continue;
      if (f.type === "select" && f.options?.length) {
        vals[f.key] = f.value || f.options[0];
      } else {
        vals[f.key] = (f.type !== "secret" && f.value) ? f.value : "";
      }
    }
    return vals;
  }, [visibleSchema]);
  const [values, setValues] = useState<Record<string, string>>(initialValues);
  // Reset form values when the schema (i.e. instance) changes — without this,
  // switching from "edit instance 0" to "edit instance 1" would keep the
  // previous instance's typed values in the inputs.
  useEffect(() => {
    setValues(initialValues);
  }, [initialValues]);

  const setValue = (key: string, val: string) => setValues(prev => ({ ...prev, [key]: val }));

  const controlField = useMemo(
    () => visibleSchema.find(f => f.type === "select" && f.options),
    [visibleSchema],
  );
  const controlValue = controlField ? (values[controlField.key] || "") : "";

  const filteredFields = useMemo(
    () => visibleSchema.filter(f => !f.show_when || f.show_when === controlValue),
    [visibleSchema, controlValue],
  );

  const handleSubmit = () => {
    const payload: Record<string, string> = {};
    for (const f of filteredFields) {
      if (f.readonly) continue;
      const v = values[f.key];
      if (v) payload[f.key] = v;
    }
    onSubmit(payload);
  };

  return (
    <div className="p-6">
      {description && <p className="text-xs text-text-dim mb-5">{description}</p>}

      {filteredFields.length > 0 ? (
        <div className="space-y-3 mb-6 max-h-80 overflow-y-auto">
          {filteredFields.map((field) => (
            <div key={field.key}>
              <label className="text-xs font-bold text-text-dim mb-1 block">
                {field.label || field.key} {field.required && <span className="text-error">*</span>}
                {field.type === "secret" && field.env_var && (
                  <span className="ml-2 font-mono text-[10px] text-text-dim/80 normal-case">{field.env_var}</span>
                )}
              </label>
              {field.readonly ? (
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={field.value || field.placeholder || ""}
                    readOnly
                    className="flex-1 rounded-lg border border-border-subtle bg-main/50 px-3 py-2 text-xs text-text-dim font-mono"
                  />
                  <button
                    onClick={async () => {
                      const ok = await copyToClipboard(field.value || field.placeholder || "");
                      addToast(ok ? t("common.copied") : t("common.copy_failed"), ok ? "success" : "error");
                    }}
                    className="px-3 py-2 rounded-lg bg-brand/10 text-brand text-xs hover:bg-brand/20 transition-colors shrink-0"
                    title={t("common.copy")}
                  >
                    {t("common.copy")}
                  </button>
                </div>
              ) : field.type === "select" && field.options ? (
                <select
                  value={values[field.key] || ""}
                  onChange={(e) => setValue(field.key, e.target.value)}
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
                >
                  {field.options.map((opt) => (
                    <option key={opt} value={opt}>{opt}</option>
                  ))}
                </select>
              ) : (
                <input
                  type={field.type === "secret" ? "password" : "text"}
                  value={values[field.key] || ""}
                  onChange={(e) => setValue(field.key, e.target.value)}
                  placeholder={field.has_value ? "••••••••  (leave empty to keep)" : (field.placeholder || field.env_var || field.key)}
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs focus:border-brand focus:ring-1 focus:ring-brand/20 outline-none"
                />
              )}
            </div>
          ))}
        </div>
      ) : (
        <div className="mb-6 p-4 rounded-lg bg-main/30 text-center">
          <p className="text-xs text-text-dim">{t("channels.no_fields_required")}</p>
        </div>
      )}

      <div className="flex gap-3">
        <Button variant="secondary" className="flex-1" onClick={onCancel} disabled={isPending}>
          {t("common.cancel")}
        </Button>
        <Button
          variant="primary"
          className="flex-1"
          onClick={handleSubmit}
          disabled={isPending}
        >
          {isPending ? t("common.saving") : submitLabel}
        </Button>
      </div>
      {/* Channel name pinned for context */}
      <p className="mt-3 text-[10px] text-text-dim/60 text-center">
        {channel.display_name || channel.name}
      </p>
    </div>
  );
}

// Multi-instance manager (#4837). Replaces the legacy single-form
// `ConfigDialog` for any non-QR channel. Three internal phases:
//   - "list" (default): shows configured instances + "Add instance" CTA
//   - "create": embedded `ChannelForm` driving `useCreateChannelInstance`
//   - "edit": embedded `ChannelForm` driving `useUpdateChannelInstance`
// Delete uses an inline confirm because the destructive action is small and
// a separate modal would be overkill.
function InstancesDialog({
  channel,
  onClose,
  t,
}: {
  channel: Channel;
  onClose: () => void;
  t: (key: string, opts?: { defaultValue?: string; count?: number }) => string;
}) {
  const addToast = useUIStore((s) => s.addToast);
  const [phase, setPhase] = useState<"list" | "create" | "edit">("list");
  const [editIndex, setEditIndex] = useState<number | null>(null);
  const [pendingDelete, setPendingDelete] = useState<number | null>(null);

  const instancesQuery = useChannelInstances(channel.name);
  const createMut = useCreateChannelInstance();
  const updateMut = useUpdateChannelInstance();
  const deleteMut = useDeleteChannelInstance();

  const instances: ChannelInstance[] = instancesQuery.data?.items ?? [];
  const channelLabel = channel.display_name || channel.name;

  const handleCreate = useCallback(
    (payload: Record<string, string>) => {
      createMut.mutate(
        { channelName: channel.name, fields: payload },
        {
          onSuccess: () => {
            addToast(
              t("channels.instance_added", { defaultValue: `${channelLabel} instance added` }),
              "success",
            );
            setPhase("list");
          },
          onError: (err) =>
            addToast(
              toastErr(err, t("channels.config_failed", { defaultValue: "Failed to save instance" })),
              "error",
            ),
        },
      );
    },
    [createMut, channel.name, channelLabel, addToast, t],
  );

  const handleUpdate = useCallback(
    (payload: Record<string, string>) => {
      if (editIndex === null) return;
      const target = instances[editIndex];
      if (!target) {
        // Defensive: the list refetch races with the edit form's submit.
        // If the row vanished, surface the 409-equivalent inline instead of
        // sending an unsigned PUT.
        addToast(
          t("channels.instance_stale", {
            defaultValue: "Instance was removed by another tab; refresh and retry",
          }),
          "error",
        );
        setPhase("list");
        setEditIndex(null);
        return;
      }
      updateMut.mutate(
        {
          channelName: channel.name,
          index: editIndex,
          fields: payload,
          signature: target.signature,
        },
        {
          onSuccess: () => {
            addToast(
              t("channels.instance_updated", { defaultValue: `${channelLabel} instance updated` }),
              "success",
            );
            setPhase("list");
            setEditIndex(null);
          },
          onError: (err) =>
            addToast(
              toastErr(err, t("channels.config_failed", { defaultValue: "Failed to save instance" })),
              "error",
            ),
        },
      );
    },
    [updateMut, channel.name, channelLabel, editIndex, instances, addToast, t],
  );

  const handleDelete = useCallback(
    (idx: number) => {
      const target = instances[idx];
      if (!target) {
        addToast(
          t("channels.instance_stale", {
            defaultValue: "Instance was removed by another tab; refresh and retry",
          }),
          "error",
        );
        setPendingDelete(null);
        return;
      }
      deleteMut.mutate(
        { channelName: channel.name, index: idx, signature: target.signature },
        {
          onSuccess: () => {
            addToast(
              t("channels.instance_removed", { defaultValue: `${channelLabel} instance removed` }),
              "success",
            );
            setPendingDelete(null);
          },
          onError: (err) => {
            addToast(
              toastErr(err, t("common.error", { defaultValue: "Error" })),
              "error",
            );
            setPendingDelete(null);
          },
        },
      );
    },
    [deleteMut, channel.name, channelLabel, instances, addToast, t],
  );

  const fieldsForEdit: ChannelField[] = useMemo(() => {
    if (phase !== "edit" || editIndex === null) return [];
    const inst = instances[editIndex];
    return inst?.fields ?? channel.fields ?? [];
  }, [phase, editIndex, instances, channel.fields]);

  const renderInstanceLabel = (inst: ChannelInstance, idx: number) => {
    // Prefer a meaningful identifier from the instance's config: either the
    // env-var name a secret field points at (e.g. `TELEGRAM_BOT_TOKEN_2`)
    // or the first non-empty stringy field. Falls back to the index.
    const obj = inst.config ?? {};
    const candidates: string[] = [];
    for (const f of channel.fields ?? []) {
      const v = obj[f.key];
      if (typeof v === "string" && v.trim() !== "") {
        candidates.push(`${f.label || f.key}: ${v}`);
      }
    }
    return candidates[0] || t("channels.instance_n", { defaultValue: `Instance #${idx}`, count: idx });
  };

  const headerTitle = phase === "create"
    ? t("channels.add_instance", { defaultValue: "Add instance" })
    : phase === "edit"
      ? t("channels.edit_instance", { defaultValue: "Edit instance" })
      : channelLabel;

  const headerSub = phase === "list"
    ? t("channels.manage_instances", { defaultValue: "Manage configured instances" })
    : channelLabel;

  return (
    <DrawerPanel isOpen onClose={onClose} size="md" hideCloseButton>
      <div className="px-6 py-5 border-b border-border-subtle">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3 min-w-0">
            {phase !== "list" && (
              <button
                onClick={() => {
                  setPhase("list");
                  setEditIndex(null);
                }}
                className="p-1.5 rounded-lg hover:bg-main transition-colors shrink-0"
                aria-label={t("common.back", { defaultValue: "Back" })}
              >
                <ArrowLeft className="w-4 h-4" />
              </button>
            )}
            <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
              <Settings className="w-5 h-5 text-brand" />
            </div>
            <div className="min-w-0">
              <h3 className="text-base font-black truncate">{headerTitle}</h3>
              <p className="text-[10px] text-text-dim mt-0.5 truncate">{headerSub}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-xl hover:bg-main transition-colors shrink-0"
            aria-label={t("common.close", { defaultValue: "Close" })}
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      {phase === "list" && (
        <div className="p-6">
          <p className="text-xs text-text-dim mb-4">
            {channel.description}
          </p>

          {instancesQuery.isLoading ? (
            <div className="space-y-2 mb-4">
              {[1, 2].map((i) => <div key={i} className="h-14 rounded-lg bg-main/30 animate-pulse" />)}
            </div>
          ) : instances.length === 0 ? (
            <div className="mb-4 p-4 rounded-lg bg-main/30 text-center">
              <p className="text-xs text-text-dim">
                {t("channels.no_instances", { defaultValue: "No instances configured yet." })}
              </p>
            </div>
          ) : (
            <div className="space-y-2 mb-4">
              {instances.map((inst, idx) => {
                const label = renderInstanceLabel(inst, idx);
                const isDeleting = pendingDelete === idx;
                return (
                  <div
                    key={idx}
                    className="flex items-center gap-3 p-3 rounded-lg border border-border-subtle bg-main/30"
                  >
                    <div className="w-7 h-7 rounded-md bg-brand/10 grid place-items-center text-brand text-xs font-bold shrink-0">
                      {idx + 1}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="font-mono text-[12px] text-text-main truncate">{label}</div>
                      <div className="font-mono text-[10px] text-text-dim mt-0.5 flex items-center gap-1.5">
                        {inst.has_token ? (
                          <><CheckCircle2 className="w-3 h-3 text-success" /> {t("channels.creds_ok", { defaultValue: "credentials set" })}</>
                        ) : (
                          <><AlertCircle className="w-3 h-3 text-warning" /> {t("channels.creds_missing", { defaultValue: "credentials missing" })}</>
                        )}
                      </div>
                    </div>
                    {isDeleting ? (
                      <div className="flex gap-1 shrink-0">
                        <button
                          onClick={() => handleDelete(idx)}
                          disabled={deleteMut.isPending}
                          className="px-2 py-1 rounded-md bg-error/10 text-error text-[10px] font-bold hover:bg-error/20"
                        >
                          {t("common.confirm", { defaultValue: "Confirm" })}
                        </button>
                        <button
                          onClick={() => setPendingDelete(null)}
                          className="px-2 py-1 rounded-md bg-main/50 text-text-dim text-[10px] font-bold hover:bg-main"
                        >
                          {t("common.cancel", { defaultValue: "Cancel" })}
                        </button>
                      </div>
                    ) : (
                      <div className="flex gap-1 shrink-0">
                        <button
                          onClick={() => {
                            setEditIndex(idx);
                            setPhase("edit");
                          }}
                          className="p-1.5 rounded-md text-text-dim hover:text-text-main hover:bg-main/40"
                          aria-label={t("common.edit", { defaultValue: "Edit" })}
                          title={t("common.edit", { defaultValue: "Edit" })}
                        >
                          <Pencil className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={() => setPendingDelete(idx)}
                          className="p-1.5 rounded-md text-text-dim hover:text-error hover:bg-error/10"
                          aria-label={t("common.delete", { defaultValue: "Delete" })}
                          title={t("common.delete", { defaultValue: "Delete" })}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          <Button
            variant="primary"
            className="w-full"
            onClick={() => setPhase("create")}
            leftIcon={<Plus className="w-4 h-4" />}
          >
            {t("channels.add_instance", { defaultValue: "Add instance" })}
          </Button>
        </div>
      )}

      {phase === "create" && (
        <ChannelForm
          channel={channel}
          fields={channel.fields ?? []}
          description={channel.description}
          submitLabel={t("common.create", { defaultValue: "Create" })}
          isPending={createMut.isPending}
          onSubmit={handleCreate}
          onCancel={() => setPhase("list")}
          t={t}
        />
      )}

      {phase === "edit" && editIndex !== null && (
        <ChannelForm
          channel={channel}
          fields={fieldsForEdit}
          description={channel.description}
          submitLabel={t("common.save", { defaultValue: "Save" })}
          isPending={updateMut.isPending}
          onSubmit={handleUpdate}
          onCancel={() => {
            setPhase("list");
            setEditIndex(null);
          }}
          t={t}
        />
      )}
    </DrawerPanel>
  );
}

// QR Login Dialog for channels with setup_type === "qr" (e.g. WeChat, WhatsApp)
function QrLoginDialog({ channel, onClose, t }: { channel: Channel; onClose: () => void; t: (key: string) => string }) {
  const configureChannelMutation = useConfigureChannel();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const cancelledRef = useRef(false);
  const pollIdRef = useRef(0);
  const [phase, setPhase] = useState<"idle" | "loading" | "scanning" | "success" | "error">("idle");
  const [message, setMessage] = useState("");

  useEffect(() => () => { cancelledRef.current = true; pollIdRef.current += 1; }, []);

  const startQr = useCallback(async () => {
    const pollId = ++pollIdRef.current;
    cancelledRef.current = false;
    setPhase("loading");
    setMessage("");
    try {
      // QR start/status are imperative long-poll probes, so they stay raw instead of going through cached query hooks.
      const qrStart = channel.name === "whatsapp" ? whatsappQrStart : wechatQrStart;
      const qrStatus = channel.name === "whatsapp" ? whatsappQrStatus : wechatQrStatus;
      const displayName = channel.name === "whatsapp" ? "WhatsApp" : "WeChat";

      const res = await qrStart();
      if (!res.available || !res.qr_code) {
        setPhase("error");
        setMessage(res.message || t("channels.qr_failed"));
        return;
      }
      setPhase("scanning");
      setMessage(res.message || `Scan this QR code with your ${displayName} app`);

      // Render QR code to canvas — use the full URL so the app recognises the scan
      const qrContent = res.qr_url || res.qr_code;
      if (canvasRef.current && qrContent) {
        QRCode.toCanvas(canvasRef.current, qrContent, { width: 256, margin: 2 });
      }

      // Serial long-poll: wait for each request to finish before sending the next.
      // The backend holds each request ~30s (long-poll), so setInterval would
      // stack up parallel requests that all resolve at once on scan → flashing UI.
      const pollLoop = async () => {
        let retries = 0;
        while (!cancelledRef.current && pollIdRef.current === pollId) {
          try {
            const status = await qrStatus(res.qr_code!);
            if (cancelledRef.current || pollIdRef.current !== pollId) break;
            if (status.connected && status.bot_token) {
              cancelledRef.current = true;
              try {
                await configureChannelMutation.mutateAsync({
                  channelName: channel.name,
                  config: { bot_token_env: status.bot_token },
                });
              } catch (error) {
                setPhase("error");
                setMessage(error instanceof Error ? error.message : t("channels.qr_failed"));
                return;
              }
              setPhase("success");
              setMessage(t("channels.login_success"));
              setTimeout(onClose, 1500);
              return;
            } else if (status.expired) {
              cancelledRef.current = true;
              setPhase("error");
              setMessage(status.message || "QR code expired");
              return;
            }
          } catch {
            // Transient error — wait briefly then retry
            if (cancelledRef.current || pollIdRef.current !== pollId) break;
            retries += 1;
            if (retries >= 15) {
              setPhase("error");
              setMessage(t("channels.qr_failed") || "Connection lost. Please try again.");
              return;
            }
            await new Promise(r => setTimeout(r, 3000));
          }
        }
      };
      pollLoop();
    } catch (e) {
      setPhase("error");
      setMessage(e instanceof Error ? e.message : "Failed to start QR login");
    }
  }, [channel.name, configureChannelMutation, onClose, t]);

  // Auto-start on mount
  useEffect(() => { startQr(); }, [startQr]);

  return (
    <DrawerPanel isOpen onClose={onClose} size="md" hideCloseButton>
        <div className="px-6 py-5 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center text-brand text-sm font-bold">
                {channel.icon || "QR"}
              </div>
              <div>
                <h3 className="text-base font-black">{channel.display_name || channel.name}</h3>
                <p className="text-[10px] text-text-dim mt-0.5">{t("channels.qr_login") || "QR Code Login"}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 rounded-xl hover:bg-main transition-colors" aria-label={t("common.close")}><X className="w-4 h-4" /></button>
          </div>
        </div>

        <div className="p-6 flex flex-col items-center gap-4">
          {phase === "loading" && (
            <div className="w-64 h-64 flex items-center justify-center bg-main/30 rounded-xl">
              <div className="animate-spin w-8 h-8 border-2 border-brand border-t-transparent rounded-full" />
            </div>
          )}

          {phase === "scanning" && (
            <div className="bg-white rounded-xl p-2">
              <canvas ref={canvasRef} />
            </div>
          )}

          {phase === "success" && (
            <div className="w-64 h-64 flex items-center justify-center bg-success/10 rounded-xl">
              <CheckCircle2 className="w-16 h-16 text-success" />
            </div>
          )}

          {phase === "error" && (
            <div className="w-64 h-64 flex flex-col items-center justify-center bg-error/10 rounded-xl gap-3">
              <XCircle className="w-16 h-16 text-error" />
              <Button variant="secondary" onClick={startQr}>{t("common.retry") || "Retry"}</Button>
            </div>
          )}

          <p className="text-xs text-text-dim text-center max-w-xs">{message}</p>
        </div>

        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
    </DrawerPanel>
  );
}

export function ChannelsPage() {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [detailsChannel, setDetailsChannel] = useState<Channel | null>(null);
  const [configuringChannel, setConfiguringChannel] = useState<Channel | null>(null);
  const [qrLoginChannel, setQrLoginChannel] = useState<Channel | null>(null);
  // The picker drawer holds the catalog of unconfigured channel types
  // (slack / discord / email / …). Default view shows only configured
  // channels so the page stays focused on what's actually wired up.
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerSearch, setPickerSearch] = useState("");

  const addToast = useUIStore((s) => s.addToast);

  const channelsQuery = useChannels();
  const testMut = useTestChannel();
  const reloadMut = useReloadChannels();

  const handleTest = (name: string) => {
    testMut.mutate(name, {
      onSuccess: () => addToast(t("channels.test_success", { defaultValue: `Channel "${name}" test passed` }), "success"),
      onError: (err) => addToast(toastErr(err, t("channels.test_failed", { defaultValue: `Channel "${name}" test failed` })), "error"),
    });
  };
  const handleReload = () => {
    reloadMut.mutate(undefined, {
      onSuccess: () => addToast(t("channels.reload_success", { defaultValue: "Channels reloaded" }), "success"),
      onError: (err) => addToast(toastErr(err, t("common.error")), "error"),
    });
  };
  const closeQrLogin = useCallback(() => setQrLoginChannel(null), []);
  const handleCardConfigure = useCallback((ch: Channel) => {
    if (ch.setup_type === "qr") setQrLoginChannel(ch);
    else setConfiguringChannel(ch);
  }, []);

  const channels = channelsQuery.data ?? [];
  const configuredCount = useMemo(() => channels.filter(c => c.configured).length, [channels]);
  const unconfiguredCount = channels.length - configuredCount;

  // Configured channels are the main page content. Filter/sort applies
  // to those only; the unconfigured catalog lives behind the Add picker.
  const filteredChannels = useMemo(
    () => [...channels]
      .filter(c => {
        if (!c.configured) return false;
        const searchMatch = !search || (c.display_name || c.name).toLowerCase().includes(search.toLowerCase()) || c.category?.toLowerCase().includes(search.toLowerCase());
        return searchMatch;
      })
      .sort((a, b) => {
        let cmp = 0;
        if (sortField === "name") cmp = a.name.localeCompare(b.name);
        else if (sortField === "category") cmp = (a.category || "").localeCompare(b.category || "");
        return sortOrder === "asc" ? cmp : -cmp;
      }),
    [channels, search, sortField, sortOrder],
  );

  // Catalog of unconfigured channel types, surfaced in the Add picker.
  const pickerChannels = useMemo(
    () => [...channels]
      .filter(c => !c.configured)
      .filter(c => !pickerSearch
        || (c.display_name || c.name).toLowerCase().includes(pickerSearch.toLowerCase())
        || c.category?.toLowerCase().includes(pickerSearch.toLowerCase()))
      .sort((a, b) => (a.display_name || a.name).localeCompare(b.display_name || b.name)),
    [channels, pickerSearch],
  );

  const openPicker = () => {
    setPickerSearch("");
    setPickerOpen(true);
  };
  const handlePick = (ch: Channel) => {
    setPickerOpen(false);
    if (ch.setup_type === "qr") setQrLoginChannel(ch);
    else setConfiguringChannel(ch);
  };

  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortOrder(sortOrder === "asc" ? "desc" : "asc");
    } else {
      setSortField(field);
      setSortOrder("asc");
    }
  };

  const handleSelect = useCallback((name: string, checked: boolean) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (checked) next.add(name);
      else next.delete(name);
      return next;
    });
  }, []);

  const handleSelectAll = () => {
    if (selectedIds.size === filteredChannels.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(filteredChannels.map(c => c.name)));
    }
  };

  const allSelected = filteredChannels.length > 0 && selectedIds.size === filteredChannels.length;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("channels.title")}
        subtitle={t("channels.subtitle")}
        isFetching={channelsQuery.isFetching}
        onRefresh={() => void channelsQuery.refetch()}
        icon={<Network className="h-4 w-4" />}
        helpText={t("channels.help")}
        actions={
          <div className="flex items-center gap-2">
            <Button variant="secondary" size="sm" onClick={handleReload} disabled={reloadMut.isPending}>
              {t("channels.reload", { defaultValue: "Reload" })}
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={openPicker}
              leftIcon={<Plus className="h-3.5 w-3.5" />}
              disabled={unconfiguredCount === 0}
              title={unconfiguredCount === 0
                ? t("channels.all_configured", { defaultValue: "All channels configured" })
                : t("channels.add", { defaultValue: "Add channel" })}
            >
              {t("channels.add", { defaultValue: "Add" })}
            </Button>
            <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
              {t("channels.configured_count", { count: configuredCount })}
            </div>
          </div>
        }
      />

      {/* Search & Controls */}
      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex-1">
          <Input
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIds(new Set()); }}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={search && (
              <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search", { defaultValue: "Clear search" })}>
                <X className="w-3 h-3" />
              </button>
            )}
          />
        </div>

        <div className="flex gap-2 items-center flex-wrap">
          {/* Sort buttons */}
          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            <button
              onClick={() => handleSort("name")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "name" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {t("channels.name")}
            </button>
            <button
              onClick={() => handleSort("category")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${sortField === "category" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              {t("channels.category")}
            </button>
          </div>

          {/* View toggle */}
          <div className="flex gap-1 p-1 bg-main/30 rounded-lg">
            <button
              onClick={() => setViewMode("grid")}
              className={`p-1.5 rounded-md transition-colors ${viewMode === "grid" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              <Grid3X3 className="w-4 h-4" />
            </button>
            <button
              onClick={() => setViewMode("list")}
              className={`p-1.5 rounded-md transition-colors ${viewMode === "list" ? "bg-surface shadow-sm" : "text-text-dim hover:text-text-main"}`}
            >
              <List className="w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      <div>
      {channelsQuery.isLoading ? (
        <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
          {[1, 2, 3].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : configuredCount === 0 ? (
        // No channels configured yet — surface the picker as a primary
        // CTA instead of a tab buried below. Mirrors the design canvas
        // empty state ("Connect Slack, Discord, email, or SMS so agents
        // can post and receive messages.").
        <Card padding="lg" className="flex flex-col items-center text-center gap-4 py-10">
          <div className="w-12 h-12 rounded-xl bg-brand/10 border border-brand/30 grid place-items-center text-brand">
            <Network className="h-6 w-6" />
          </div>
          <div className="max-w-md space-y-2">
            <h2 className="text-base font-bold text-text-main">
              {t("channels.empty_title", { defaultValue: "No channels yet" })}
            </h2>
            <p className="text-sm text-text-dim leading-relaxed">
              {t("channels.empty_body", {
                defaultValue: "Connect Slack, Discord, email, SMS, or any of the bundled bridges so agents can post and receive messages.",
              })}
            </p>
          </div>
          <Button variant="primary" size="md" onClick={openPicker} leftIcon={<Plus className="h-4 w-4" />}>
            {t("channels.connect_first", { defaultValue: "Connect a channel" })}
          </Button>
        </Card>
      ) : filteredChannels.length === 0 ? (
        <EmptyState
          title={search ? t("channels.no_results") : t("channels.no_configured")}
          icon={<Search className="h-6 w-6" />}
        />
      ) : (
        <>
          {/* Select all */}
          <div className="flex items-center gap-2">
            <button
              onClick={handleSelectAll}
              className="flex items-center gap-2 text-xs font-bold text-text-dim hover:text-text-main transition-colors"
            >
              {allSelected ? <CheckSquare className="w-4 h-4 text-brand" /> : <Square className="w-4 h-4" />}
              {t("channels.select_all")}
            </button>
            {search && (
              <span className="text-xs text-text-dim">({filteredChannels.length} {t("channels.results")})</span>
            )}
          </div>

          <div className={viewMode === "grid" ? "grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 3xl:grid-cols-5 4xl:grid-cols-6" : "flex flex-col gap-2"}>
            {filteredChannels.map((c) => (
              <ChannelCard
                key={c.name}
                channel={c}
                isSelected={selectedIds.has(c.name)}
                viewMode={viewMode}
                onSelect={handleSelect}
                onConfigure={handleCardConfigure}
                onViewDetails={setDetailsChannel}
                t={t}
              />
            ))}
          </div>
        </>
      )}
      </div>

      {/* Details Modal */}
      {detailsChannel && (
        <DetailsModal
          channel={detailsChannel}
          onClose={() => setDetailsChannel(null)}
          onConfigure={() => {
            const ch = detailsChannel;
            setDetailsChannel(null);
            if (ch.setup_type === "qr") {
              setQrLoginChannel(ch);
            } else {
              setConfiguringChannel(ch);
            }
          }}
          onTest={() => handleTest(detailsChannel.name)}
          t={t}
        />
      )}

      {/* Instance manager dialog (#4837) — drives create / edit / delete
          for `[[channels.<name>]]` array entries. Replaces the legacy
          single-form ConfigDialog for non-QR channels. */}
      {configuringChannel && (
        <InstancesDialog
          key={configuringChannel?.name}
          channel={configuringChannel}
          onClose={() => setConfiguringChannel(null)}
          t={t}
        />
      )}

      {/* QR Login Dialog */}
      {qrLoginChannel && (
        <QrLoginDialog
          channel={qrLoginChannel}
          onClose={closeQrLogin}
          t={t}
        />
      )}

      {/* Add-channel picker — shows the catalog of unconfigured channel
          types. Click one to launch the existing configure / QR flow. */}
      <DrawerPanel
        isOpen={pickerOpen}
        onClose={() => setPickerOpen(false)}
        title={t("channels.picker_title", { defaultValue: "Add channel" })}
        size="lg"
      >
        <div className="flex flex-col gap-4">
          <Input
            value={pickerSearch}
            onChange={(e) => setPickerSearch(e.target.value)}
            placeholder={t("common.search")}
            leftIcon={<Search className="w-4 h-4" />}
            rightIcon={pickerSearch && (
              <button
                onClick={() => setPickerSearch("")}
                className="hover:text-text-main"
                aria-label={t("common.clear_search", { defaultValue: "Clear search" })}
              >
                <X className="w-3 h-3" />
              </button>
            )}
          />
          {pickerChannels.length === 0 ? (
            <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
              {pickerSearch
                ? t("channels.no_results")
                : t("channels.all_configured", { defaultValue: "All available channel types are already configured." })}
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
              {pickerChannels.map((c) => (
                <button
                  key={c.name}
                  type="button"
                  onClick={() => handlePick(c)}
                  className="flex items-center gap-3 px-3 py-2.5 rounded-lg border border-border-subtle bg-main/40 hover:border-brand/40 hover:bg-main/60 transition-colors text-left"
                >
                  <div className="w-9 h-9 rounded-lg bg-brand/10 border border-brand/20 grid place-items-center text-brand shrink-0">
                    {getChannelIcon(c.name)}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="font-mono text-[13px] font-medium text-text-main truncate">
                      {c.display_name || c.name}
                    </div>
                    <div className="font-mono text-[10.5px] text-text-dim/80 truncate">
                      {c.category || c.name}
                    </div>
                  </div>
                  <ChevronRight className="w-4 h-4 text-text-dim shrink-0" />
                </button>
              ))}
            </div>
          )}
        </div>
      </DrawerPanel>
    </div>
  );
}
