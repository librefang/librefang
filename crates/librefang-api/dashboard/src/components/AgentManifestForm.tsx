import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ChevronDown, Plus, Trash2, X } from "lucide-react";
import { generateUid } from "../lib/agentManifest";
import type { ManifestExtras, ManifestFormState } from "../lib/agentManifest";

interface AgentManifestFormProps {
  value: ManifestFormState;
  onChange: (next: ManifestFormState) => void;
  providers: { name: string }[];
  models: { provider: string; id: string }[];
  invalidFields: Set<string>;
  // Read-only view of preserved-but-not-form-renderable extras. We show
  // a hint next to dropdowns whose form widget can't represent the
  // contents (e.g. a full `[exec_policy]` table) so the user isn't
  // misled by a default-looking dropdown that hides serialized state.
  extras: ManifestExtras;
}

export function AgentManifestForm({
  value,
  onChange,
  providers,
  models,
  invalidFields,
  extras,
}: AgentManifestFormProps) {
  const { t } = useTranslation();

  // Curried setters for the nested-state update boilerplate.
  const update = (patch: Partial<ManifestFormState>): void => onChange({ ...value, ...patch });
  const updateModel = (patch: Partial<ManifestFormState["model"]>): void =>
    onChange({ ...value, model: { ...value.model, ...patch } });
  const updateResources = (patch: Partial<ManifestFormState["resources"]>): void =>
    onChange({ ...value, resources: { ...value.resources, ...patch } });
  const updateCapabilities = (patch: Partial<ManifestFormState["capabilities"]>): void =>
    onChange({ ...value, capabilities: { ...value.capabilities, ...patch } });
  const updateThinking = (patch: Partial<ManifestFormState["thinking"]>): void =>
    onChange({ ...value, thinking: { ...value.thinking, ...patch } });
  const updateAutonomous = (patch: Partial<ManifestFormState["autonomous"]>): void =>
    onChange({ ...value, autonomous: { ...value.autonomous, ...patch } });
  const updateRouting = (patch: Partial<ManifestFormState["routing"]>): void =>
    onChange({ ...value, routing: { ...value.routing, ...patch } });

  const filteredModels = useMemo(
    () => (value.model.provider ? models.filter((m) => m.provider === value.model.provider) : models),
    [models, value.model.provider],
  );

  const jsonSchemaFormat =
    value.response_format.mode === "json_schema" ? value.response_format : null;

  return (
    <div className="space-y-4">
      <Section title={t("agents.form.basics")}>
        <Field label={t("agents.form.name")} required invalid={invalidFields.has("name")}>
          <input
            type="text"
            value={value.name}
            onChange={(e) => update({ name: e.target.value })}
            placeholder={t("agents.form.name_placeholder")}
            className={inputClass}
            autoFocus
          />
        </Field>
        <Field label={t("agents.form.description")}>
          <input
            type="text"
            value={value.description}
            onChange={(e) => update({ description: e.target.value })}
            placeholder={t("agents.form.description_placeholder")}
            className={inputClass}
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.version")}>
            <input
              type="text"
              value={value.version}
              onChange={(e) => update({ version: e.target.value })}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.author")}>
            <input
              type="text"
              value={value.author}
              onChange={(e) => update({ author: e.target.value })}
              className={inputClass}
            />
          </Field>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.module")}>
            <input
              type="text"
              value={value.module}
              onChange={(e) => update({ module: e.target.value })}
              placeholder={t("agents.form.module_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.priority")}>
            <select
              value={value.priority}
              onChange={(e) => update({ priority: e.target.value as ManifestFormState["priority"] })}
              className={inputClass}
            >
              <option value="Low">{t("agents.form.priority_low")}</option>
              <option value="Normal">{t("agents.form.priority_normal")}</option>
              <option value="High">{t("agents.form.priority_high")}</option>
              <option value="Critical">{t("agents.form.priority_critical")}</option>
            </select>
          </Field>
        </div>
      </Section>

      <Section title={t("agents.form.model")}>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.provider")} required invalid={invalidFields.has("model.provider")}>
            <select
              value={value.model.provider}
              onChange={(e) => updateModel({ provider: e.target.value, model: "" })}
              className={inputClass}
            >
              <option value="">{t("agents.form.select_provider")}</option>
              {providers.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("agents.form.model_id")} required invalid={invalidFields.has("model.model")}>
            {filteredModels.length > 0 ? (
              <select
                value={value.model.model}
                onChange={(e) => updateModel({ model: e.target.value })}
                className={inputClass}
              >
                <option value="">{t("agents.form.select_model")}</option>
                {filteredModels.map((m) => (
                  <option key={`${m.provider}/${m.id}`} value={m.id}>
                    {m.id}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={value.model.model}
                onChange={(e) => updateModel({ model: e.target.value })}
                placeholder={t("agents.form.model_id_placeholder")}
                className={inputClass}
              />
            )}
          </Field>
        </div>
        <Field label={t("agents.form.system_prompt")}>
          <textarea
            value={value.model.system_prompt}
            onChange={(e) => updateModel({ system_prompt: e.target.value })}
            placeholder={t("agents.form.system_prompt_placeholder")}
            rows={3}
            className={textareaClass}
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.temperature")}>
            <input
              type="number"
              step="0.1"
              min="0"
              max="2"
              value={value.model.temperature}
              onChange={(e) => updateModel({ temperature: e.target.value })}
              placeholder={t("agents.form.temperature_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.max_tokens")}>
            <input
              type="number"
              min="1"
              value={value.model.max_tokens}
              onChange={(e) => updateModel({ max_tokens: e.target.value })}
              placeholder={t("agents.form.max_tokens_placeholder")}
              className={inputClass}
            />
          </Field>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.api_key_env")} hint={t("agents.form.api_key_env_hint")}>
            <input
              type="text"
              value={value.model.api_key_env}
              onChange={(e) => updateModel({ api_key_env: e.target.value })}
              placeholder={t("agents.form.api_key_env_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.base_url")}>
            <input
              type="text"
              value={value.model.base_url}
              onChange={(e) => updateModel({ base_url: e.target.value })}
              placeholder={t("agents.form.base_url_placeholder")}
              className={inputClass}
            />
          </Field>
        </div>
      </Section>

      <Section title={t("agents.form.resources")}>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.tokens_per_hour")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_llm_tokens_per_hour}
              onChange={(e) => updateResources({ max_llm_tokens_per_hour: e.target.value })}
              placeholder={t("agents.form.inherit_default")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.tool_calls_per_minute")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_tool_calls_per_minute}
              onChange={(e) => updateResources({ max_tool_calls_per_minute: e.target.value })}
              placeholder={t("agents.form.tool_calls_per_minute_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.cost_per_hour")}>
            <input
              type="number"
              step="0.01"
              min="0"
              value={value.resources.max_cost_per_hour_usd}
              onChange={(e) => updateResources({ max_cost_per_hour_usd: e.target.value })}
              placeholder={t("agents.form.unlimited_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.cost_per_day")}>
            <input
              type="number"
              step="0.01"
              min="0"
              value={value.resources.max_cost_per_day_usd}
              onChange={(e) => updateResources({ max_cost_per_day_usd: e.target.value })}
              placeholder={t("agents.form.unlimited_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.cost_per_month")}>
            <input
              type="number"
              step="0.01"
              min="0"
              value={value.resources.max_cost_per_month_usd}
              onChange={(e) => updateResources({ max_cost_per_month_usd: e.target.value })}
              placeholder={t("agents.form.unlimited_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.network_bytes_per_hour")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_network_bytes_per_hour}
              onChange={(e) => updateResources({ max_network_bytes_per_hour: e.target.value })}
              placeholder={t("agents.form.network_bytes_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.memory_bytes")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_memory_bytes}
              onChange={(e) => updateResources({ max_memory_bytes: e.target.value })}
              placeholder={t("agents.form.memory_bytes_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.cpu_time_ms")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_cpu_time_ms}
              onChange={(e) => updateResources({ max_cpu_time_ms: e.target.value })}
              placeholder={t("agents.form.cpu_time_placeholder")}
              className={inputClass}
            />
          </Field>
        </div>
      </Section>

      <Section title={t("agents.form.capabilities")}>
        <Field label={t("agents.form.network_hosts")} hint={t("agents.form.network_hosts_hint")}>
          <TagInput
            value={value.capabilities.network}
            onChange={(next) => updateCapabilities({ network: next })}
            placeholder={t("agents.form.network_hosts_placeholder")}
          />
        </Field>
        <Field label={t("agents.form.shell_commands")} hint={t("agents.form.shell_commands_hint")}>
          <TagInput
            value={value.capabilities.shell}
            onChange={(next) => updateCapabilities({ shell: next })}
            placeholder={t("agents.form.shell_commands_placeholder")}
          />
        </Field>
        <Field label={t("agents.form.cap_tools")} hint={t("agents.form.cap_tools_hint")}>
          <TagInput
            value={value.capabilities.tools}
            onChange={(next) => updateCapabilities({ tools: next })}
            placeholder={t("agents.form.cap_tools_placeholder")}
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.memory_read")}>
            <TagInput
              value={value.capabilities.memory_read}
              onChange={(next) => updateCapabilities({ memory_read: next })}
              placeholder={t("agents.form.memory_glob_placeholder")}
            />
          </Field>
          <Field label={t("agents.form.memory_write")}>
            <TagInput
              value={value.capabilities.memory_write}
              onChange={(next) => updateCapabilities({ memory_write: next })}
              placeholder={t("agents.form.memory_glob_placeholder")}
            />
          </Field>
          <Field label={t("agents.form.agent_message")}>
            <TagInput
              value={value.capabilities.agent_message}
              onChange={(next) => updateCapabilities({ agent_message: next })}
              placeholder={t("agents.form.agent_message_placeholder")}
            />
          </Field>
          <Field label={t("agents.form.ofp_connect")}>
            <TagInput
              value={value.capabilities.ofp_connect}
              onChange={(next) => updateCapabilities({ ofp_connect: next })}
              placeholder={t("agents.form.ofp_connect_placeholder")}
            />
          </Field>
        </div>
        <div className="flex flex-wrap gap-4 pt-1">
          <Toggle
            label={t("agents.form.agent_spawn")}
            checked={value.capabilities.agent_spawn}
            onChange={(checked) => updateCapabilities({ agent_spawn: checked })}
          />
          <Toggle
            label={t("agents.form.ofp_discover")}
            checked={value.capabilities.ofp_discover}
            onChange={(checked) => updateCapabilities({ ofp_discover: checked })}
          />
        </div>
      </Section>

      <Section title={t("agents.form.discovery")}>
        <Field label={t("agents.form.tags")}>
          <TagInput
            value={value.tags}
            onChange={(next) => update({ tags: next })}
            placeholder={t("agents.form.tags_placeholder")}
          />
        </Field>
        <Field label={t("agents.form.skills")}>
          <TagInput
            value={value.skills}
            onChange={(next) => update({ skills: next })}
            placeholder={t("agents.form.skills_placeholder")}
          />
        </Field>
        <Field label={t("agents.form.mcp_servers")}>
          <TagInput
            value={value.mcp_servers}
            onChange={(next) => update({ mcp_servers: next })}
            placeholder={t("agents.form.mcp_servers_placeholder")}
          />
        </Field>
      </Section>

      <CollapsibleSection title={t("agents.form.scheduling")} defaultOpen={false}>
        <Field label={t("agents.form.schedule_mode")} hint={t("agents.form.schedule_mode_hint")}>
          <select
            value={value.schedule.mode}
            onChange={(e) => {
              const mode = e.target.value as ManifestFormState["schedule"]["mode"];
              if (mode === "reactive") update({ schedule: { mode } });
              else if (mode === "periodic") update({ schedule: { mode, cron: "" } });
              else if (mode === "proactive") update({ schedule: { mode, conditions: [] } });
              else update({ schedule: { mode, check_interval_secs: "300" } });
            }}
            className={inputClass}
          >
            <option value="reactive">{t("agents.form.schedule_reactive")}</option>
            <option value="periodic">{t("agents.form.schedule_periodic")}</option>
            <option value="proactive">{t("agents.form.schedule_proactive")}</option>
            <option value="continuous">{t("agents.form.schedule_continuous")}</option>
          </select>
        </Field>
        {value.schedule.mode === "periodic" && (
          <Field label={t("agents.form.cron")} hint={t("agents.form.cron_hint")}>
            <input
              type="text"
              value={value.schedule.cron}
              onChange={(e) => update({ schedule: { mode: "periodic", cron: e.target.value } })}
              placeholder={t("agents.form.cron_placeholder")}
              className={inputClass}
            />
          </Field>
        )}
        {value.schedule.mode === "proactive" && (
          <Field label={t("agents.form.conditions")}>
            <TagInput
              value={value.schedule.conditions}
              onChange={(next) => update({ schedule: { mode: "proactive", conditions: next } })}
              placeholder={t("agents.form.conditions_placeholder")}
            />
          </Field>
        )}
        {value.schedule.mode === "continuous" && (
          <Field label={t("agents.form.check_interval_secs")}>
            <input
              type="number"
              min="1"
              value={value.schedule.check_interval_secs}
              onChange={(e) =>
                update({
                  schedule: { mode: "continuous", check_interval_secs: e.target.value },
                })
              }
              placeholder={t("agents.form.check_interval_placeholder")}
              className={inputClass}
            />
          </Field>
        )}
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.fallback_models")} defaultOpen={false}>
        <p className="text-[10px] text-text-dim/70 mb-2">{t("agents.form.fallback_models_hint")}</p>
        {value.fallback_models.map((fb, idx) => (
          <div
            key={fb._uid}
            className="rounded-lg border border-border-subtle/60 bg-main/40 p-2 mb-2 space-y-2"
          >
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-bold text-text-dim uppercase">#{idx + 1}</span>
              <button
                type="button"
                onClick={() => update({ fallback_models: value.fallback_models.filter((_, i) => i !== idx) })}
                className="text-text-dim hover:text-error"
                aria-label="remove fallback"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <input
                type="text"
                value={fb.provider}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models, idx, { ...fb, provider: e.target.value }) })}
                placeholder={t("agents.form.provider")}
                className={inputClass}
              />
              <input
                type="text"
                value={fb.model}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models, idx, { ...fb, model: e.target.value }) })}
                placeholder={t("agents.form.model_id")}
                className={inputClass}
              />
              <input
                type="text"
                value={fb.api_key_env}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models, idx, { ...fb, api_key_env: e.target.value }) })}
                placeholder={t("agents.form.api_key_env")}
                className={inputClass}
              />
              <input
                type="text"
                value={fb.base_url}
                onChange={(e) => update({ fallback_models: patchListItem(value.fallback_models, idx, { ...fb, base_url: e.target.value }) })}
                placeholder={t("agents.form.base_url")}
                className={inputClass}
              />
            </div>
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            update({
              fallback_models: [
                ...value.fallback_models,
                { _uid: generateUid(), provider: "", model: "", api_key_env: "", base_url: "", extras: {} },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.add_fallback")}
        </button>
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.thinking")} defaultOpen={false}>
        <Toggle
          label={t("agents.form.thinking_enabled")}
          checked={value.thinking.enabled}
          onChange={(checked) => updateThinking({ enabled: checked })}
        />
        {value.thinking.enabled && (
          <div className="grid grid-cols-2 gap-3 mt-2">
            <Field label={t("agents.form.budget_tokens")}>
              <input
                type="number"
                min="0"
                value={value.thinking.budget_tokens}
                onChange={(e) => updateThinking({ budget_tokens: e.target.value })}
                placeholder={t("agents.form.budget_tokens_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.stream_thinking")}>
              <Toggle
                label=""
                checked={value.thinking.stream_thinking}
                onChange={(checked) => updateThinking({ stream_thinking: checked })}
              />
            </Field>
          </div>
        )}
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.autonomous")} defaultOpen={false}>
        <Toggle
          label={t("agents.form.autonomous_enabled")}
          checked={value.autonomous.enabled}
          onChange={(checked) => updateAutonomous({ enabled: checked })}
        />
        {value.autonomous.enabled && (
          <div className="grid grid-cols-2 gap-3 mt-2">
            <Field label={t("agents.form.max_iterations")}>
              <input
                type="number"
                min="1"
                value={value.autonomous.max_iterations}
                onChange={(e) => updateAutonomous({ max_iterations: e.target.value })}
                placeholder={t("agents.form.max_iterations_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.max_restarts")}>
              <input
                type="number"
                min="0"
                value={value.autonomous.max_restarts}
                onChange={(e) => updateAutonomous({ max_restarts: e.target.value })}
                placeholder={t("agents.form.max_restarts_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.heartbeat_interval_secs")}>
              <input
                type="number"
                min="1"
                value={value.autonomous.heartbeat_interval_secs}
                onChange={(e) => updateAutonomous({ heartbeat_interval_secs: e.target.value })}
                placeholder={t("agents.form.heartbeat_interval_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.heartbeat_timeout_secs")}>
              <input
                type="number"
                min="1"
                value={value.autonomous.heartbeat_timeout_secs}
                onChange={(e) => updateAutonomous({ heartbeat_timeout_secs: e.target.value })}
                placeholder={t("agents.form.auto_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.heartbeat_keep_recent")}>
              <input
                type="number"
                min="0"
                value={value.autonomous.heartbeat_keep_recent}
                onChange={(e) => updateAutonomous({ heartbeat_keep_recent: e.target.value })}
                placeholder={t("agents.form.auto_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.heartbeat_channel")}>
              <input
                type="text"
                value={value.autonomous.heartbeat_channel}
                onChange={(e) => updateAutonomous({ heartbeat_channel: e.target.value })}
                placeholder={t("agents.form.heartbeat_channel_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.quiet_hours")} hint={t("agents.form.quiet_hours_hint")}>
              <input
                type="text"
                value={value.autonomous.quiet_hours}
                onChange={(e) => updateAutonomous({ quiet_hours: e.target.value })}
                placeholder={t("agents.form.quiet_hours_placeholder")}
                className={inputClass}
              />
            </Field>
          </div>
        )}
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.routing")} defaultOpen={false}>
        <Toggle
          label={t("agents.form.routing_enabled")}
          checked={value.routing.enabled}
          onChange={(checked) => updateRouting({ enabled: checked })}
        />
        {value.routing.enabled && (
          <div className="space-y-2 mt-2">
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("agents.form.simple_model")}>
                <input
                  type="text"
                  value={value.routing.simple_model}
                  onChange={(e) => updateRouting({ simple_model: e.target.value })}
                  className={inputClass}
                />
              </Field>
              <Field label={t("agents.form.medium_model")}>
                <input
                  type="text"
                  value={value.routing.medium_model}
                  onChange={(e) => updateRouting({ medium_model: e.target.value })}
                  className={inputClass}
                />
              </Field>
              <Field label={t("agents.form.complex_model")}>
                <input
                  type="text"
                  value={value.routing.complex_model}
                  onChange={(e) => updateRouting({ complex_model: e.target.value })}
                  className={inputClass}
                />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field label={t("agents.form.simple_threshold")}>
                <input
                  type="number"
                  min="0"
                  value={value.routing.simple_threshold}
                  onChange={(e) => updateRouting({ simple_threshold: e.target.value })}
                  placeholder={t("agents.form.simple_threshold_placeholder")}
                  className={inputClass}
                />
              </Field>
              <Field label={t("agents.form.complex_threshold")}>
                <input
                  type="number"
                  min="0"
                  value={value.routing.complex_threshold}
                  onChange={(e) => updateRouting({ complex_threshold: e.target.value })}
                  placeholder={t("agents.form.complex_threshold_placeholder")}
                  className={inputClass}
                />
              </Field>
            </div>
          </div>
        )}
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.context_injection")} defaultOpen={false}>
        <p className="text-[10px] text-text-dim/70 mb-2">
          {t("agents.form.context_injection_hint")}
        </p>
        {value.context_injection.map((ci, idx) => (
          <div
            key={ci._uid}
            className="rounded-lg border border-border-subtle/60 bg-main/40 p-2 mb-2 space-y-2"
          >
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-bold text-text-dim uppercase">#{idx + 1}</span>
              <button
                type="button"
                onClick={() => update({ context_injection: value.context_injection.filter((_, i) => i !== idx) })}
                className="text-text-dim hover:text-error"
                aria-label="remove context injection"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <input
                type="text"
                value={ci.name}
                onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, name: e.target.value }) })}
                placeholder={t("agents.form.injection_name")}
                className={inputClass}
              />
              <select
                value={ci.position}
                onChange={(e) => update({
                  context_injection: patchListItem(value.context_injection, idx, {
                    ...ci,
                    position: e.target.value as ManifestFormState["context_injection"][number]["position"],
                  }),
                })}
                className={inputClass}
              >
                <option value="system">{t("agents.form.position_system")}</option>
                <option value="before_user">{t("agents.form.position_before_user")}</option>
                <option value="after_reset">{t("agents.form.position_after_reset")}</option>
              </select>
            </div>
            <textarea
              value={ci.content}
              onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, content: e.target.value }) })}
              placeholder={t("agents.form.injection_content")}
              rows={2}
              className={textareaClass}
            />
            <input
              type="text"
              value={ci.condition}
              onChange={(e) => update({ context_injection: patchListItem(value.context_injection, idx, { ...ci, condition: e.target.value }) })}
              placeholder={t("agents.form.injection_condition")}
              className={inputClass}
            />
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            update({
              context_injection: [
                ...value.context_injection,
                { _uid: generateUid(), name: "", content: "", position: "system", condition: "" },
              ],
            })
          }
          className="flex items-center gap-1 text-xs text-brand hover:underline"
        >
          <Plus className="w-3.5 h-3.5" />
          {t("agents.form.add_injection")}
        </button>
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.response_format")} defaultOpen={false}>
        {value.response_format.mode === "text" && extras.topLevel.response_format !== undefined && (
          <ExtrasOverrideHint message={t("agents.form.response_format_extras_hint")} />
        )}
        <Field label={t("agents.form.response_format_mode")}>
          <select
            value={value.response_format.mode}
            onChange={(e) => {
              const mode = e.target.value as ManifestFormState["response_format"]["mode"];
              if (mode === "text") update({ response_format: { mode } });
              else if (mode === "json") update({ response_format: { mode } });
              else update({ response_format: { mode, name: "", schema: "{}", strict: false } });
            }}
            className={inputClass}
          >
            <option value="text">{t("agents.form.response_text")}</option>
            <option value="json">{t("agents.form.response_json")}</option>
            <option value="json_schema">{t("agents.form.response_json_schema")}</option>
          </select>
        </Field>
        {jsonSchemaFormat && (
          <div className="space-y-2 mt-2">
            <Field label={t("agents.form.schema_name")}>
              <input
                type="text"
                value={jsonSchemaFormat.name}
                onChange={(e) =>
                  update({
                    response_format: {
                      mode: "json_schema",
                      name: e.target.value,
                      schema: jsonSchemaFormat.schema,
                      strict: jsonSchemaFormat.strict,
                    },
                  })
                }
                placeholder={t("agents.form.schema_name_placeholder")}
                className={inputClass}
              />
            </Field>
            <Field label={t("agents.form.schema_body")}>
              <textarea
                value={jsonSchemaFormat.schema}
                onChange={(e) =>
                  update({
                    response_format: {
                      mode: "json_schema",
                      name: jsonSchemaFormat.name,
                      schema: e.target.value,
                      strict: jsonSchemaFormat.strict,
                    },
                  })
                }
                rows={6}
                className={textareaClass}
              />
            </Field>
            <Toggle
              label={t("agents.form.strict")}
              checked={jsonSchemaFormat.strict}
              onChange={(checked) =>
                update({
                  response_format: {
                    mode: "json_schema",
                    name: jsonSchemaFormat.name,
                    schema: jsonSchemaFormat.schema,
                    strict: checked,
                  },
                })
              }
            />
          </div>
        )}
      </CollapsibleSection>

      <CollapsibleSection title={t("agents.form.lifecycle")} defaultOpen={false}>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("agents.form.session_mode")}>
            <select
              value={value.session_mode}
              onChange={(e) =>
                update({ session_mode: e.target.value as ManifestFormState["session_mode"] })
              }
              className={inputClass}
            >
              <option value="persistent">{t("agents.form.session_persistent")}</option>
              <option value="new">{t("agents.form.session_new")}</option>
            </select>
          </Field>
          <Field label={t("agents.form.web_search_aug")}>
            <select
              value={value.web_search_augmentation}
              onChange={(e) =>
                update({
                  web_search_augmentation:
                    e.target.value as ManifestFormState["web_search_augmentation"],
                })
              }
              className={inputClass}
            >
              <option value="off">{t("agents.form.web_search_off")}</option>
              <option value="auto">{t("agents.form.web_search_auto")}</option>
              <option value="always">{t("agents.form.web_search_always")}</option>
            </select>
          </Field>
          <Field
            label={t("agents.form.exec_policy")}
            hint={
              !value.exec_policy_shorthand && extras.topLevel.exec_policy !== undefined
                ? t("agents.form.exec_policy_extras_hint")
                : undefined
            }
          >
            <select
              value={value.exec_policy_shorthand}
              onChange={(e) =>
                update({
                  exec_policy_shorthand:
                    e.target.value as ManifestFormState["exec_policy_shorthand"],
                })
              }
              className={inputClass}
            >
              <option value="">{t("agents.form.exec_policy_global")}</option>
              <option value="allow">allow</option>
              <option value="deny">deny</option>
              <option value="full">full</option>
              <option value="allowlist">allowlist</option>
            </select>
          </Field>
          <Field label={t("agents.form.pinned_model")}>
            <input
              type="text"
              value={value.pinned_model}
              onChange={(e) => update({ pinned_model: e.target.value })}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.workspace")}>
            <input
              type="text"
              value={value.workspace}
              onChange={(e) => update({ workspace: e.target.value })}
              placeholder={t("agents.form.auto_placeholder")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.allowed_plugins")}>
            <TagInput
              value={value.allowed_plugins}
              onChange={(next) => update({ allowed_plugins: next })}
              placeholder={t("agents.form.allowed_plugins_placeholder")}
            />
          </Field>
        </div>
        <div className="flex flex-wrap gap-4 pt-2">
          <Toggle
            label={t("agents.form.enabled")}
            checked={value.enabled}
            onChange={(checked) => update({ enabled: checked })}
          />
          <Toggle
            label={t("agents.form.skills_disabled")}
            checked={value.skills_disabled}
            onChange={(checked) => update({ skills_disabled: checked })}
          />
          <Toggle
            label={t("agents.form.tools_disabled")}
            checked={value.tools_disabled}
            onChange={(checked) => update({ tools_disabled: checked })}
          />
          <Toggle
            label={t("agents.form.inherit_parent_context")}
            checked={value.inherit_parent_context}
            onChange={(checked) => update({ inherit_parent_context: checked })}
          />
          <Toggle
            label={t("agents.form.generate_identity_files")}
            checked={value.generate_identity_files}
            onChange={(checked) => update({ generate_identity_files: checked })}
          />
        </div>
      </CollapsibleSection>
    </div>
  );
}

const inputClass =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

const textareaClass = `${inputClass} resize-y font-mono text-xs`;

/** Patch a single item in an immutable list. Pass an object to replace, or a function to transform. */
function patchListItem<T>(list: T[], idx: number, patch: T | ((item: T) => T)): T[] {
  const next = list.slice();
  next[idx] = typeof patch === "function" ? (patch as (item: T) => T)(next[idx]) : patch;
  return next;
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2.5 rounded-xl border border-border-subtle/60 bg-surface/40 p-3">
      <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">{title}</p>
      {children}
    </div>
  );
}

function CollapsibleSection({
  title,
  children,
  defaultOpen,
}: {
  title: string;
  children: React.ReactNode;
  defaultOpen?: boolean;
}) {
  return (
    <details
      className="group rounded-xl border border-border-subtle/60 bg-surface/40 overflow-hidden"
      open={defaultOpen}
    >
      <summary
        className="flex items-center justify-between p-3 cursor-pointer list-none select-none"
      >
        <span className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
          {title}
        </span>
        <ChevronDown className="w-4 h-4 text-text-dim transition-transform group-open:rotate-180" />
      </summary>
      <div className="px-3 pb-3 space-y-2.5">{children}</div>
    </details>
  );
}

function Field({
  label,
  hint,
  required,
  invalid,
  children,
}: {
  label: string;
  hint?: string;
  required?: boolean;
  invalid?: boolean;
  children: React.ReactNode;
}) {
  // Wrap children inside the <label> rather than relying on htmlFor —
  // implicit association works without each child needing an id, and
  // clicking the label text focuses the input as users expect.
  return (
    <label className="block">
      {label && (
        <span
          className={`text-[10px] font-bold uppercase block ${
            invalid ? "text-error" : "text-text-dim"
          }`}
        >
          {label}
          {required && <span className="ml-0.5 text-error">*</span>}
        </span>
      )}
      <span className={label ? "mt-1 block" : "block"}>{children}</span>
      {hint && <span className="mt-1 text-[10px] text-text-dim/70 block">{hint}</span>}
    </label>
  );
}

function ExtrasOverrideHint({ message }: { message: string }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 px-2.5 py-1.5 text-[11px] text-warning">
      <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
      <span>{message}</span>
    </div>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-xs cursor-pointer select-none">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="h-4 w-4 rounded border-border-subtle accent-brand"
      />
      {label}
    </label>
  );
}

function TagInput({
  value,
  onChange,
  placeholder,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
}) {
  const [inputValue, setInputValue] = useState("");

  const commit = (raw: string): void => {
    const cleaned = raw.trim();
    if (!cleaned) return;
    if (value.includes(cleaned)) return;
    onChange([...value, cleaned]);
    setInputValue("");
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border-subtle bg-main px-2 py-1.5 focus-within:border-brand">
      {value.map((tag) => (
        <span
          key={tag}
          className="inline-flex items-center gap-1 rounded-md bg-surface px-1.5 py-0.5 text-[11px] font-medium text-text"
        >
          {tag}
          <button
            type="button"
            onClick={() => onChange(value.filter((t) => t !== tag))}
            className="text-text-dim hover:text-error"
            aria-label={`remove ${tag}`}
          >
            <X className="h-3 w-3" />
          </button>
        </span>
      ))}
      <input
        type="text"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        placeholder={value.length === 0 ? placeholder : undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit(inputValue);
          } else if (e.key === "Backspace" && !inputValue && value.length > 0) {
            onChange(value.slice(0, -1));
          }
        }}
        onBlur={() => {
          if (inputValue) {
            commit(inputValue);
          }
        }}
        className="flex-1 min-w-[100px] bg-transparent text-xs outline-none placeholder:text-text-dim/40"
      />
    </div>
  );
}
