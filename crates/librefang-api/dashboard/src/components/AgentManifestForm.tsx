import { useId, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import type { ManifestFormState } from "../lib/agentManifest";

interface AgentManifestFormProps {
  value: ManifestFormState;
  onChange: (next: ManifestFormState) => void;
  providers: { name: string }[];
  models: { provider: string; id: string }[];
  invalidFields: Set<string>;
}

export function AgentManifestForm({
  value,
  onChange,
  providers,
  models,
  invalidFields,
}: AgentManifestFormProps) {
  const { t } = useTranslation();

  const update = (patch: Partial<ManifestFormState>): void => {
    onChange({ ...value, ...patch });
  };
  const updateModel = (patch: Partial<ManifestFormState["model"]>): void => {
    onChange({ ...value, model: { ...value.model, ...patch } });
  };
  const updateResources = (patch: Partial<ManifestFormState["resources"]>): void => {
    onChange({ ...value, resources: { ...value.resources, ...patch } });
  };
  const updateCapabilities = (patch: Partial<ManifestFormState["capabilities"]>): void => {
    onChange({ ...value, capabilities: { ...value.capabilities, ...patch } });
  };

  const filteredModels = useMemo(
    () =>
      value.model.provider
        ? models.filter((m) => m.provider === value.model.provider)
        : models,
    [models, value.model.provider],
  );

  return (
    <div className="space-y-4">
      <Section title={t("agents.form.basics")}>
        <Field label={t("agents.form.name")} required invalid={invalidFields.has("name")}>
          <input
            type="text"
            value={value.name}
            onChange={(e) => update({ name: e.target.value })}
            placeholder="researcher"
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
      </Section>

      <Section title={t("agents.form.model")}>
        <div className="grid grid-cols-2 gap-3">
          <Field
            label={t("agents.form.provider")}
            required
            invalid={invalidFields.has("model.provider")}
          >
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
          <Field
            label={t("agents.form.model_id")}
            required
            invalid={invalidFields.has("model.model")}
          >
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
                placeholder="gpt-4o"
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
            className={`${inputClass} resize-y font-mono text-xs`}
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
              placeholder="0.7"
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.max_tokens")}>
            <input
              type="number"
              min="1"
              value={value.model.max_tokens}
              onChange={(e) => updateModel({ max_tokens: e.target.value })}
              placeholder="4096"
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
              onChange={(e) =>
                updateResources({ max_llm_tokens_per_hour: e.target.value })
              }
              placeholder={t("agents.form.inherit_default")}
              className={inputClass}
            />
          </Field>
          <Field label={t("agents.form.tool_calls_per_minute")}>
            <input
              type="number"
              min="0"
              value={value.resources.max_tool_calls_per_minute}
              onChange={(e) =>
                updateResources({ max_tool_calls_per_minute: e.target.value })
              }
              placeholder="60"
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
              placeholder="0 = unlimited"
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
              placeholder="0 = unlimited"
              className={inputClass}
            />
          </Field>
        </div>
      </Section>

      <Section title={t("agents.form.capabilities")}>
        <Field
          label={t("agents.form.network_hosts")}
          hint={t("agents.form.network_hosts_hint")}
        >
          <TagInput
            value={value.capabilities.network}
            onChange={(next) => updateCapabilities({ network: next })}
            placeholder="api.openai.com:443"
          />
        </Field>
        <Field
          label={t("agents.form.shell_commands")}
          hint={t("agents.form.shell_commands_hint")}
        >
          <TagInput
            value={value.capabilities.shell}
            onChange={(next) => updateCapabilities({ shell: next })}
            placeholder="ls, cat, grep"
          />
        </Field>
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
          <Toggle
            label={t("agents.form.enabled")}
            checked={value.enabled}
            onChange={(checked) => update({ enabled: checked })}
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
    </div>
  );
}

const inputClass =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2.5 rounded-xl border border-border-subtle/60 bg-surface/40 p-3">
      <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
        {title}
      </p>
      {children}
    </div>
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
  const id = useId();
  return (
    <div>
      <label
        htmlFor={id}
        className={`text-[10px] font-bold uppercase ${
          invalid ? "text-error" : "text-text-dim"
        }`}
      >
        {label}
        {required && <span className="ml-0.5 text-error">*</span>}
      </label>
      <div className="mt-1">{children}</div>
      {hint && <p className="mt-1 text-[10px] text-text-dim/70">{hint}</p>}
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

// Lightweight chip input — comma or Enter commits a tag, Backspace on
// an empty buffer pops the last one. We avoid MultiSelectCmdk because
// these fields are open-ended (free-form host strings, command names,
// arbitrary tags) rather than picks from a known set.
function TagInput({
  value,
  onChange,
  placeholder,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
}) {
  const commit = (raw: string): void => {
    const cleaned = raw.trim();
    if (!cleaned) return;
    if (value.includes(cleaned)) return;
    onChange([...value, cleaned]);
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
        placeholder={value.length === 0 ? placeholder : undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit(e.currentTarget.value);
            e.currentTarget.value = "";
          } else if (e.key === "Backspace" && !e.currentTarget.value && value.length > 0) {
            onChange(value.slice(0, -1));
          }
        }}
        onBlur={(e) => {
          if (e.currentTarget.value) {
            commit(e.currentTarget.value);
            e.currentTarget.value = "";
          }
        }}
        className="flex-1 min-w-[100px] bg-transparent text-xs outline-none placeholder:text-text-dim/40"
      />
    </div>
  );
}
