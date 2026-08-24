/**
 * Agent binding control for one workflow step, extracted out of
 * `pages/CanvasPage.tsx`'s node config panel (refs #7724).
 *
 * A step's agent can be bound two ways, and the API accepts both:
 * `agent_id` pins a specific running instance, `agent_name` resolves by
 * name at run time and therefore survives a respawn that mints a new id.
 * The chosen source lives in explicit state (`StepAgentBindingValue.source`)
 * and each source renders only its own control.
 *
 * That shape is the point. The tempting alternative — keep one `<select>`
 * and force its controlled value to `""` whenever the other field is set —
 * builds a one-way door: the select can never report the value that would
 * clear the other field, so the binding cannot be switched back. Rendering
 * exactly the control that belongs to the active source means no control is
 * ever fed a value contradicting its own state, and both directions of the
 * switch are reachable.
 *
 * The switch is also lossless: leaving `instance` carries the selected
 * agent's name into the name field, and returning re-selects the live agent
 * that carries that name when one exists.
 */
import type { AgentItem } from "../api";
import type { CanvasNodeData } from "../lib/canvas";
import { CANVAS_INPUT_CLASS, CANVAS_LABEL_CLASS } from "../lib/canvas";

/** Which field of the step's `agent` binding the operator is authoring. */
export type StepAgentSource = "instance" | "name";

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
  sessionMode: StepSessionMode;
};

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
 * from whichever field the backend actually populated — `agent_id` wins
 * because that is the precedence the API's step parser applies.
 */
export function bindingFromNodeData(data: CanvasNodeData): StepAgentBindingValue {
  const agentId = typeof data.agentId === "string" ? data.agentId : "";
  const agentName = typeof data.agentName === "string" ? data.agentName : "";
  const stored = data.agentSource;
  const source: StepAgentSource =
    stored === "name" || stored === "instance"
      ? stored
      : !agentId && agentName
        ? "name"
        : "instance";
  return { source, agentId, agentName, sessionMode: normalizeSessionMode(data.sessionMode) };
}

/**
 * Project the editing state back onto node data.
 *
 * Exactly one of `agentId` / `agentName` survives, so `buildSteps` can emit
 * the matching API field without re-deriving the operator's intent, and a
 * stale value from the other source can never leak into a saved step.
 */
export function bindingToNodeData(
  value: StepAgentBindingValue,
  agents: AgentItem[],
): Pick<CanvasNodeData, "agentSource" | "agentId" | "agentName" | "sessionMode"> {
  const sessionMode = value.sessionMode === "" ? undefined : value.sessionMode;
  if (value.source === "name") {
    const name = value.agentName.trim();
    return { agentSource: "name", agentId: undefined, agentName: name || undefined, sessionMode };
  }
  const agent = agents.find(a => a.id === value.agentId);
  return {
    agentSource: "instance",
    agentId: value.agentId || undefined,
    agentName: agent?.name || undefined,
    sessionMode,
  };
}

/**
 * The `agent_*` / `session_mode` fields of the API step payload for a node.
 *
 * The API's step parser reads `agent_id` first, so a payload carrying both
 * fields makes the name binding unreachable; exactly the field the operator
 * chose is emitted. `session_mode` is omitted rather than sent empty — the
 * parser logs and discards a value it cannot read, and an unset override is
 * how a step defers to the target agent's manifest.
 */
export function stepAgentFields(data: CanvasNodeData): {
  agent_id?: string;
  agent_name?: string;
  session_mode?: "persistent" | "new";
} {
  const agentId = typeof data.agentId === "string" ? data.agentId : "";
  const agentName = typeof data.agentName === "string" ? data.agentName.trim() : "";
  const sessionMode = normalizeSessionMode(data.sessionMode);
  const byName = data.agentSource === "name" || !agentId;
  return {
    agent_id: byName ? undefined : agentId,
    agent_name: byName ? agentName || undefined : undefined,
    session_mode: sessionMode === "" ? undefined : sessionMode,
  };
}

/** True when the step has an agent the backend can resolve. A name-only
 *  binding counts — it is a valid `agent_name` step. */
export function isStepBound(data: CanvasNodeData): boolean {
  return !!data.agentId || !!(typeof data.agentName === "string" && data.agentName.trim());
}

/** Move the binding to another source without losing the operator's work. */
export function switchAgentSource(
  value: StepAgentBindingValue,
  next: StepAgentSource,
  agents: AgentItem[],
): StepAgentBindingValue {
  if (next === value.source) return value;
  if (next === "name") {
    const selected = agents.find(a => a.id === value.agentId);
    return { ...value, source: "name", agentName: selected?.name ?? value.agentName };
  }
  // Returning to a concrete instance: re-select the live agent carrying the
  // typed name so `name -> instance -> name` is a no-op, and fall back to
  // "no agent" (an empty, still-selectable dropdown) when none matches.
  const match = agents.find(a => a.name === value.agentName.trim());
  return {
    ...value,
    source: "instance",
    agentId: match?.id ?? "",
    agentName: match?.name ?? value.agentName,
  };
}

export function StepAgentBinding({
  value,
  agents,
  onChange,
  t,
}: {
  value: StepAgentBindingValue;
  agents: AgentItem[];
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
        onChange={e => onChange(switchAgentSource(value, e.target.value as StepAgentSource, agents))}
        className={CANVAS_INPUT_CLASS}
      >
        <option value="instance">{t("canvas.agent_source_instance")}</option>
        <option value="name">{t("canvas.agent_source_name")}</option>
      </select>

      {value.source === "instance" ? (
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
      ) : (
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
    </div>
  );
}
