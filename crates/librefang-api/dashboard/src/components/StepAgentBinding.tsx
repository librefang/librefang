/**
 * Agent binding control for one workflow step, extracted out of
 * `pages/CanvasPage.tsx`'s node config panel (refs #7724).
 *
 * A step's agent can be bound three ways, and the API accepts all three:
 * `agent_id` pins a specific running instance, `agent_name` resolves by
 * name at run time and therefore survives a respawn that mints a new id,
 * and `agent_type` resolves find-or-spawn against an agent template,
 * spawning one when nothing of that type is running (#7712).
 * The chosen source lives in explicit state (`StepAgentBindingValue.source`)
 * and each source renders only its own control.
 *
 * That shape is the point. The tempting alternative — keep one `<select>`
 * and force its controlled value to `""` whenever the other field is set —
 * builds a one-way door: the select can never report the value that would
 * clear the other field, so the binding cannot be switched back. Rendering
 * exactly the control that belongs to the active source means no control is
 * ever fed a value contradicting its own state, and every direction of the
 * switch is reachable.
 *
 * The source is three-valued rather than a boolean for that same reason: a
 * step bound to a type would otherwise be a door with no handle on the
 * inside, since nothing in the panel could clear `agentType` again.
 *
 * The switch is also lossless: whichever name is on screen — the selected
 * agent's name, a typed name, or a template name — is carried into the next
 * source, and returning to `instance` re-selects the live agent that
 * carries that name when one exists.
 */
import type { AgentItem } from "../api";
import type { CanvasNodeData } from "../lib/canvas";
import { CANVAS_INPUT_CLASS, CANVAS_LABEL_CLASS, stepAgentPayload } from "../lib/canvas";

/** Which field of the step's `agent` binding the operator is authoring. */
export type StepAgentSource = "instance" | "name" | "type";

const AGENT_SOURCES: readonly StepAgentSource[] = ["instance", "name", "type"];

/** Narrow a stored or user-supplied source; anything else reads as "not recorded". */
function asAgentSource(raw: unknown): StepAgentSource | null {
  return AGENT_SOURCES.includes(raw as StepAgentSource) ? (raw as StepAgentSource) : null;
}

/** Per-step `session_mode`; `""` defers to the target agent's manifest. */
export type StepSessionMode = "" | "persistent" | "new";

/** Editing state for a step's agent binding, owned by the config panel. */
export type StepAgentBindingValue = {
  source: StepAgentSource;
  /** Agent UUID — the payload when `source === "instance"`. */
  agentId: string;
  /** Agent name — the payload when `source === "name"`, and the display
   *  echo of the selected instance otherwise. */
  agentName: string;
  /** Agent template name — the payload when `source === "type"`. */
  agentType: string;
  sessionMode: StepSessionMode;
  /** Raw, comma-separated `required_skills` text exactly as typed (#7721).
   *  Kept as text rather than a parsed array so a half-typed name — the moment after a comma, a trailing space — survives a re-render instead of being normalised out from under the cursor.
   *  Parsed on projection. */
  requiredSkills: string;
};

/**
 * Parse the comma-separated `required_skills` box into the array the API takes.
 *
 * Trims, drops empties and de-duplicates, so `"a, , a"` is `["a"]` and an all-whitespace box is an empty list rather than one blank requirement — the API rejects a blank entry with a 400 rather than ignoring it, which would turn a stray comma into an unsaveable workflow.
 * Order is the operator's; the API sorts on persist.
 */
export function parseRequiredSkills(raw: string): string[] {
  const seen = new Set<string>();
  for (const part of raw.split(",")) {
    const name = part.trim();
    if (name) seen.add(name);
  }
  return [...seen];
}

/** Render a stored `required_skills` list back into the editable box.
 *  Anything that is not an array of strings reads as "none required", which is what an older canvas draft and a step with no requirement both look like. */
export function formatRequiredSkills(raw: unknown): string {
  if (!Array.isArray(raw)) return "";
  return raw.filter((name): name is string => typeof name === "string" && name.trim() !== "")
    .map(name => name.trim())
    .join(", ");
}

/** Narrow an untrusted `session_mode` to the two values the API parses.
 *  Anything else (absent, null, a typo, a number) means "defer to the
 *  manifest", which is what the API's own lenient parser does with it. */
export function normalizeSessionMode(raw: unknown): StepSessionMode {
  return raw === "persistent" || raw === "new" ? raw : "";
}

/**
 * Seed the editing state from a canvas node.
 *
 * `agentSource` is absent on every node built before this control existed
 * and on every node hydrated from a workflow's steps, so it is inferred
 * from whichever field the backend actually populated. The inference
 * follows the same specificity order as `stepAgentPayload` — id, then type,
 * then name — so the control opens on the binding the workflow would send.
 */
export function bindingFromNodeData(data: CanvasNodeData): StepAgentBindingValue {
  const agentId = typeof data.agentId === "string" ? data.agentId : "";
  const agentName = typeof data.agentName === "string" ? data.agentName : "";
  const agentType = typeof data.agentType === "string" ? data.agentType : "";
  const stored = asAgentSource(data.agentSource);
  const source: StepAgentSource =
    stored ?? (agentId ? "instance" : agentType ? "type" : agentName ? "name" : "instance");
  return {
    source,
    agentId,
    agentName,
    agentType,
    sessionMode: normalizeSessionMode(data.sessionMode),
    requiredSkills: formatRequiredSkills(data.requiredSkills),
  };
}

/**
 * Project the editing state back onto node data.
 *
 * Choosing a source writes that source's field and clears the other two, so
 * `buildSteps` can emit the matching API field without re-deriving the
 * operator's intent, a stale value from an abandoned source can never leak
 * into a saved step, and every one of the three bindings can be left again.
 */
export function bindingToNodeData(
  value: StepAgentBindingValue,
  agents: AgentItem[],
): Pick<
  CanvasNodeData,
  "agentSource" | "agentId" | "agentName" | "agentType" | "sessionMode" | "requiredSkills"
> {
  const sessionMode = value.sessionMode === "" ? undefined : value.sessionMode;
  // An empty requirement list is stored as absent rather than `[]`: the two mean the same thing to the API, and absent is what every step authored before this control existed already looks like.
  const parsedSkills = parseRequiredSkills(value.requiredSkills);
  const requiredSkills = parsedSkills.length > 0 ? parsedSkills : undefined;
  if (value.source === "name") {
    const name = value.agentName.trim();
    return {
      agentSource: "name",
      agentId: undefined,
      agentName: name || undefined,
      agentType: undefined,
      sessionMode,
      requiredSkills,
    };
  }
  if (value.source === "type") {
    const type = value.agentType.trim();
    return {
      agentSource: "type",
      agentId: undefined,
      agentName: undefined,
      agentType: type || undefined,
      sessionMode,
      requiredSkills,
    };
  }
  const agent = agents.find(a => a.id === value.agentId);
  return {
    agentSource: "instance",
    agentId: value.agentId || undefined,
    agentName: agent?.name || undefined,
    agentType: undefined,
    sessionMode,
    requiredSkills,
  };
}

/**
 * The `agent_*` / `session_mode` / `required_skills` fields of the API step payload for a node.
 *
 * The routing key comes from `stepAgentPayload`, which sends exactly one of
 * `agent_id` / `agent_name` / `agent_type`: a payload carrying several is
 * rejected by the API rather than resolved, so the field the operator chose
 * is the only one emitted. `session_mode` is omitted rather than sent empty
 * — the parser logs and discards a value it cannot read, and an unset
 * override is how a step defers to the target agent's manifest.
 *
 * `required_skills` is omitted when empty for the same reason, and because the API's parser for it is strict rather than lenient (#7721): a blank entry is a 400 naming the step, not a silently dropped requirement.
 */
export function stepAgentFields(data: CanvasNodeData): {
  agent_id?: string;
  agent_name?: string;
  agent_type?: string;
  session_mode?: "persistent" | "new";
  required_skills?: string[];
} {
  const sessionMode = normalizeSessionMode(data.sessionMode);
  const requiredSkills = formatRequiredSkills(data.requiredSkills);
  const parsedSkills = parseRequiredSkills(requiredSkills);
  return {
    ...(stepAgentPayload(data) ?? {}),
    session_mode: sessionMode === "" ? undefined : sessionMode,
    required_skills: parsedSkills.length > 0 ? parsedSkills : undefined,
  };
}

/** True when the step has an agent the backend can resolve. A name-only or
 *  type-only binding counts: `agent_name` is resolved at run time and
 *  `agent_type` is find-or-spawn, so neither is an unassigned step. */
export function isStepBound(data: CanvasNodeData): boolean {
  return stepAgentPayload(data) !== null;
}

/** Move the binding to another source without losing the operator's work. */
export function switchAgentSource(
  value: StepAgentBindingValue,
  next: StepAgentSource,
  agents: AgentItem[],
): StepAgentBindingValue {
  if (next === value.source) return value;
  // The human-readable handle for whatever is bound right now: the selected
  // instance's name, the typed agent name, or the template name. Carrying it
  // across is what makes every direction of the switch lossless.
  const handle = (
    value.source === "instance"
      ? agents.find(a => a.id === value.agentId)?.name ?? value.agentName
      : value.source === "type"
        ? value.agentType
        : value.agentName
  ).trim();
  if (next === "name") return { ...value, source: "name", agentName: handle || value.agentName };
  if (next === "type") return { ...value, source: "type", agentType: handle || value.agentType };
  // Returning to a concrete instance: re-select the live agent carrying the
  // handle so `instance -> elsewhere -> instance` is a no-op, and fall back to
  // "no agent" (an empty, still-selectable dropdown) when none matches.
  const match = agents.find(a => a.name === handle);
  return {
    ...value,
    source: "instance",
    agentId: match?.id ?? "",
    agentName: match?.name ?? handle,
  };
}

export function StepAgentBinding({
  value,
  agents,
  skills = [],
  onChange,
  t,
}: {
  value: StepAgentBindingValue;
  agents: AgentItem[];
  /** Names of the skills loaded on this instance, offered as suggestions.
   *  Suggestions only — a workflow may legitimately require a skill that is declared but not yet installed, and the dry run is what reports that. */
  skills?: string[];
  onChange: (next: StepAgentBindingValue) => void;
  t: (key: string) => string;
}) {
  return (
    <div>
      <label className={CANVAS_LABEL_CLASS} htmlFor="step-agent-source">
        {t("canvas.agent_source_label")}
      </label>
      <select
        id="step-agent-source"
        value={value.source}
        onChange={e =>
          onChange(switchAgentSource(value, asAgentSource(e.target.value) ?? "instance", agents))
        }
        className={CANVAS_INPUT_CLASS}
      >
        <option value="instance">{t("canvas.agent_source_instance")}</option>
        <option value="name">{t("canvas.agent_source_name")}</option>
        <option value="type">{t("canvas.agent_source_type")}</option>
      </select>

      {value.source === "instance" && (
        <>
          <label className={CANVAS_LABEL_CLASS} htmlFor="step-agent-instance">
            {t("canvas.assign_agent")}
          </label>
          <select
            id="step-agent-instance"
            value={value.agentId}
            onChange={e => {
              const agent = agents.find(a => a.id === e.target.value);
              onChange({
                ...value,
                agentId: e.target.value,
                agentName: agent?.name ?? "",
              });
            }}
            className={CANVAS_INPUT_CLASS}
          >
            <option value="">{t("canvas.no_agent")}</option>
            {agents.map(a => (
              <option key={a.id} value={a.id}>
                {a.name}
                {a.state === "Running" ? "" : ` (${a.state})`}
              </option>
            ))}
          </select>
        </>
      )}

      {value.source === "name" && (
        <>
          <label className={CANVAS_LABEL_CLASS} htmlFor="step-agent-name">
            {t("canvas.agent_name_label")}
          </label>
          <input
            id="step-agent-name"
            type="text"
            list="step-agent-name-options"
            value={value.agentName}
            placeholder={t("canvas.agent_name_placeholder")}
            onChange={e => onChange({ ...value, agentName: e.target.value })}
            className={CANVAS_INPUT_CLASS}
          />
          <datalist id="step-agent-name-options">
            {agents.map(a => (
              <option key={a.id} value={a.name} />
            ))}
          </datalist>
          <p className="mt-1 text-[10px] leading-snug text-text-dim/70">
            {t("canvas.agent_name_hint")}
          </p>
        </>
      )}

      {value.source === "type" && (
        <>
          <label className={CANVAS_LABEL_CLASS} htmlFor="step-agent-type">
            {t("canvas.agent_type_label")}
          </label>
          <input
            id="step-agent-type"
            type="text"
            value={value.agentType}
            placeholder={t("canvas.agent_type_placeholder")}
            onChange={e => onChange({ ...value, agentType: e.target.value })}
            className={CANVAS_INPUT_CLASS}
          />
          <p className="mt-1 text-[10px] leading-snug text-text-dim/70">
            {t("canvas.agent_type_hint")}
          </p>
        </>
      )}

      <label className={CANVAS_LABEL_CLASS} htmlFor="step-session-mode">
        {t("canvas.session_mode_label")}
      </label>
      <select
        id="step-session-mode"
        value={value.sessionMode}
        onChange={e =>
          onChange({ ...value, sessionMode: normalizeSessionMode(e.target.value) })
        }
        className={CANVAS_INPUT_CLASS}
      >
        <option value="">{t("canvas.session_mode_default")}</option>
        <option value="persistent">{t("canvas.session_mode_persistent")}</option>
        <option value="new">{t("canvas.session_mode_new")}</option>
      </select>
      <p className="mt-1 text-[10px] leading-snug text-text-dim/70">
        {t("canvas.session_mode_hint")}
      </p>

      <label className={CANVAS_LABEL_CLASS} htmlFor="step-required-skills">
        {t("canvas.required_skills_label")}
      </label>
      <input
        id="step-required-skills"
        type="text"
        list="step-required-skills-options"
        value={value.requiredSkills}
        placeholder={t("canvas.required_skills_placeholder")}
        onChange={e => onChange({ ...value, requiredSkills: e.target.value })}
        className={CANVAS_INPUT_CLASS}
      />
      <datalist id="step-required-skills-options">
        {skills.map(name => (
          <option key={name} value={name} />
        ))}
      </datalist>
      <p className="mt-1 text-[10px] leading-snug text-text-dim/70">
        {t("canvas.required_skills_hint")}
      </p>
    </div>
  );
}
