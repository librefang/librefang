import { memo, useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ChannelItem } from "../api";
import { useChannels } from "../lib/queries/channels";
import {
  useReloadChannels,
  useSaveSidecarConfig,
} from "../lib/mutations/channels";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import {
  Network, Search, CheckCircle2, ChevronRight, X, Grid3X3, List,
  Settings, AlertCircle, CheckSquare, Square, Plus,
  MessageCircle, Mail, Phone, Link2, Radio, Send, Bell, Globe
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
        <div className="font-mono text-[13px] truncate text-text-main">
          {c.display_name || c.name}
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
      {/* Sidecar channels are config.toml-managed (no /api/channels
          configure endpoint — it would 404), so suppress the inline
          Configure affordance; the whole-card click still opens the
          read-only details drawer. */}
      {c.category !== "sidecar" && (
        <button
          type="button"
          onClick={(e) => { e.stopPropagation(); onConfigure(c); }}
          className="shrink-0 p-1.5 rounded-md text-text-dim hover:text-text-main hover:bg-main/40 transition-colors"
          aria-label={t("channels.config")}
          title={t("channels.config")}
        >
          <Settings className="w-3.5 h-3.5" />
        </button>
      )}
      {viewMode === "grid" && (
        <ChevronRight className="w-4 h-4 text-text-dim/60 shrink-0" aria-hidden="true" />
      )}
    </Card>
  );
});

// Details Modal — read-only view onto a single channel. Configure /
// reload flows live on the page header + the SidecarForm drawer; this
// modal exists for "what is this thing" inspection plus the
// copy-into-config.toml snippet for unconfigured discovery rows.
function DetailsModal({ channel, onClose, t }: {
  channel: Channel;
  onClose: () => void;
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
              <div className="flex justify-between items-center p-3 rounded-lg bg-main/20">
                <span className="text-xs font-bold text-text-dim">{t("channels.has_token")}</span>
                <span className={`text-xs font-bold ${channel.has_token ? "text-success" : "text-warning"}`}>
                  {channel.has_token ? t("common.yes") : t("common.no")}
                </span>
              </div>
            </div>
          </div>

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

          {/* Every channel runs as an out-of-process sidecar. The modal
              is read-only; the save flow lives in `SidecarForm` (Plus →
              picker → schema-driven drawer, or the gear on a card). The
              copyable `config_template` snippet (also emitted by the
              backend on each row) is intentionally surfaced inside
              `SidecarForm` rather than here, since this modal only
              opens for already-configured channels. */}
          <div className="p-4 rounded-xl bg-brand/5 border border-brand/20">
            <p className="text-xs text-text-dim">
              Runs as an out-of-process sidecar adapter. Manage it in <code className="font-mono">config.toml</code> (<code className="font-mono">[[sidecar_channels]]</code>) — Config → Sidecar Channels, or use the gear on the channel card to open the configure drawer.
            </p>
          </div>
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
    </DrawerPanel>
  );
}

// Schema-driven save form for every channel (all sidecar after the
// in-process registry was retired). Sidecar adapters expose their config
// schema via `python -m <module> --describe`; the daemon caches that
// schema and surfaces `channel.fields[]` on `/api/channels`. Submit hits
// `POST /api/channels/sidecar/{name}/configure`, which splits values
// across `secrets.env` (secret-typed fields) and `config.toml`
// (everything else) — see `useSaveSidecarConfig` for the wire shape.
function SidecarForm({
  channel,
  onClose,
  t,
}: {
  channel: Channel;
  onClose: () => void;
  t: (key: string, opts?: { defaultValue?: string; keys?: string }) => string;
}) {
  const addToast = useUIStore((s) => s.addToast);
  const saveMut = useSaveSidecarConfig();
  const allFields = channel.fields ?? [];
  const fields = allFields.filter((f) => !f.advanced);
  const advanced = allFields.filter((f) => f.advanced);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const visible = showAdvanced ? [...fields, ...advanced] : fields;

  // Pre-populate from the schema:
  //  - non-secret fields with a `value` get their value
  //  - secret fields are never echoed back as plaintext, so they
  //    start empty; we surface `has_value: true` via the placeholder
  //    below ("•••• (set — leave blank to keep)") so the operator
  //    knows the slot is already filled and won't be wiped if they
  //    don't retype.
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      allFields.map((f) => [
        f.key,
        f.type !== "secret" && typeof f.value === "string" ? f.value : "",
      ]),
    ),
  );

  const handleSubmit = () => {
    // Drop empty optional values: server interprets a missing key
    // as "leave the existing value alone" (partial update). For
    // secret fields with `has_value: true`, an empty submission
    // therefore preserves the stored secret rather than clearing it.
    const payload: Record<string, string> = {};
    for (const f of allFields) {
      const v = values[f.key]?.trim();
      if (v) payload[f.key] = v;
    }
    saveMut.mutate(
      { name: channel.name, values: payload },
      {
        onSuccess: (res) => {
          addToast(
            res.restart_required
              ? t("channels.saved_restart_required", {
                  defaultValue: "Saved — restart daemon to apply",
                })
              : t("channels.saved", { defaultValue: "Saved" }),
            "success",
          );
          // Plan Risk #5: surface shell-environment shadowing of secret
          // fields. `addToast` has no "warning" variant (success | error
          // | info), so fall back to "error" with an explicit prefix —
          // visually distinct from the "Saved" success toast above, and
          // tells the operator the save *did* happen but the new value
          // is being shadowed until they unset the shell export.
          if (res.shadowed_secrets && res.shadowed_secrets.length > 0) {
            addToast(
              t("channels.shadowed_secrets_warning", {
                defaultValue:
                  "Warning: these tokens are shadowed by shell environment variables and won't take effect until you unset them and restart: {{keys}}",
                keys: res.shadowed_secrets.join(", "),
              }),
              "error",
            );
          }
          onClose();
        },
        onError: (err) =>
          addToast(toastErr(err, t("common.error", { defaultValue: "Error" })), "error"),
      },
    );
  };

  return (
    <DrawerPanel isOpen onClose={onClose} size="lg" hideCloseButton>
      <div className="h-2 bg-linear-to-r from-brand via-brand/60 to-brand/30" />
      <div className="p-6 border-b border-border-subtle flex items-center justify-between">
        <h2 className="text-xl font-black">{channel.display_name || channel.name}</h2>
        <button onClick={onClose} className="p-2" aria-label={t("common.close", { defaultValue: "Close" })}>
          <X className="w-5 h-5" />
        </button>
      </div>
      <div className="p-6 space-y-3">
        {visible.map((f) => (
          <div key={f.key} className="space-y-1">
            <label className="text-xs font-bold">
              {f.label || f.key}
              {f.required && <span className="text-error">*</span>}
              {f.type === "secret" && f.env_var && (
                <span className="ml-2 font-mono text-[10px] text-text-dim/80 normal-case">
                  {f.env_var}
                </span>
              )}
            </label>
            {f.type === "select" && f.options && f.options.length > 0 ? (
              <Select
                options={f.options.map((o) => ({ value: o, label: o }))}
                value={values[f.key] ?? ""}
                placeholder={f.placeholder ?? undefined}
                onChange={(e) =>
                  setValues((v) => ({ ...v, [f.key]: e.target.value }))
                }
              />
            ) : (
              <Input
                type={f.type === "secret" ? "password" : "text"}
                value={values[f.key] ?? ""}
                placeholder={
                  f.type === "secret" && f.has_value
                    ? t("channels.secret_set_placeholder", {
                        defaultValue: "•••• (set — leave blank to keep)",
                      })
                    : f.placeholder ?? undefined
                }
                onChange={(e) =>
                  setValues((v) => ({ ...v, [f.key]: e.target.value }))
                }
              />
            )}
          </div>
        ))}
        {advanced.length > 0 && (
          <button
            type="button"
            className="text-xs text-text-dim underline"
            onClick={() => setShowAdvanced((s) => !s)}
          >
            {showAdvanced
              ? t("common.hide_advanced", { defaultValue: "Hide advanced" })
              : t("common.show_advanced", { defaultValue: "Show advanced" })}
          </button>
        )}
        {channel.config_template && (
          <details className="pt-2">
            <summary className="text-xs text-text-dim cursor-pointer select-none">
              {t("channels.config_template_summary", {
                defaultValue: "Or paste this into config.toml by hand",
              })}
            </summary>
            <pre className="mt-2 p-3 rounded-md bg-main/30 border border-border-subtle text-[11px] font-mono text-text-main whitespace-pre overflow-x-auto select-all">
              {channel.config_template}
            </pre>
          </details>
        )}
      </div>
      <div className="p-4 border-t border-border-subtle flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={saveMut.isPending}>
          {t("common.cancel")}
        </Button>
        <Button
          variant="primary"
          onClick={handleSubmit}
          disabled={saveMut.isPending}
        >
          {saveMut.isPending
            ? t("common.saving", { defaultValue: "Saving…" })
            : t("common.save", { defaultValue: "Save" })}
        </Button>
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
  // Every channel is sidecar now (the in-process registry was removed),
  // so configure always lands on the schema-driven SidecarForm drawer.
  const [sidecarFormChannel, setSidecarFormChannel] = useState<Channel | null>(null);
  // The picker drawer holds the catalog of unconfigured channel types
  // (slack / discord / email / …). Default view shows only configured
  // channels so the page stays focused on what's actually wired up.
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerSearch, setPickerSearch] = useState("");

  const addToast = useUIStore((s) => s.addToast);

  const channelsQuery = useChannels();
  const reloadMut = useReloadChannels();

  const handleReload = () => {
    reloadMut.mutate(undefined, {
      onSuccess: () => addToast(t("channels.reload_success", { defaultValue: "Channels reloaded" }), "success"),
      onError: (err) => addToast(toastErr(err, t("common.error")), "error"),
    });
  };
  const handleCardConfigure = useCallback((ch: Channel) => {
    setSidecarFormChannel(ch);
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
    // Schema-driven save endpoint
    // (`POST /api/channels/sidecar/{name}/configure`) is the only
    // configure path now — every channel runs as a sidecar.
    setSidecarFormChannel(ch);
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

      {/* Details Modal — read-only "what is this" view. */}
      {detailsChannel && (
        <DetailsModal
          channel={detailsChannel}
          onClose={() => setDetailsChannel(null)}
          t={t}
        />
      )}

      {/* Sidecar configure form — schema-driven, hits
          `POST /api/channels/sidecar/{name}/configure`. */}
      {sidecarFormChannel && (
        <SidecarForm
          channel={sidecarFormChannel}
          onClose={() => setSidecarFormChannel(null)}
          t={t}
        />
      )}

      {/* Add-channel picker — shows the catalog of unconfigured channel
          types. Click one to launch the SidecarForm drawer. */}
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
