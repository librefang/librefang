import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Gauge, Pin } from "lucide-react";
import {
  CANONICAL_REASONING_EFFORTS,
  reasoningEffortLabel,
} from "../lib/reasoningEffort";

/**
 * The composer's reasoning-effort control: a pill in the Deep Thinking row
 * that opens the levels the active model accepts.
 *
 * Capability first: the list handed in is exactly what the active model's
 * catalog entry allows (`supported_efforts`, with the `off_allowed`/
 * `mandatory` filter applied by `lib/reasoningEffort` — a model that cannot be
 * switched off never shows `none`), and the menu never widens it. Only when
 * the caller hands in no list at all (older daemon, provider with no metadata)
 * does the canonical set stand in; an explicit empty list is the backend saying
 * the model offers no known level (#8616), not missing data. A stored level
 * outside the offered set still gets a row so
 * the operator can see what is in effect, but it is marked and disabled — the
 * session route validates the token rather than coercing it to a declared
 * level, and the UI must not suggest it.
 * (When the catalog starts marking budget-only models — reasoning shaped by a
 * token budget rather than a level enum — this control should render a budget
 * input instead of a level list. Nothing marks that today; noted so the branch
 * is deliberate when it lands.)
 *
 * Two verbs, two affordances, both spelled out in the footer:
 *
 *   - clicking a level uses it for the **next message only** — a one-shot the
 *     send path consumes (the turn carries `reasoning_effort` once), so the
 *     turn after that falls back to the session or agent default on its own;
 *   - clicking a row's pin **fixes the level to the session** (`PATCH
 *     /api/sessions/{id}/reasoning`), where it survives reloads and applies to
 *     every turn until removed from the pinned banner.
 *
 * The check mark always names the level the next message will actually use
 * (one-shot, else session pin, else Default); the pill repeats the same answer
 * with a `next` badge for a one-shot and a pin icon for a session pin.
 *
 * Presentational on purpose: the caller owns the session PATCH and the store
 * one-shot. Everything here degrades — no catalog metadata, no session field
 * from an older daemon, no pin wiring — to a usable menu.
 */
export interface ReasoningEffortMenuProps {
  /**
   * Offered levels, already capability-filtered by the caller. Absent =
   * canonical fallback; an explicit empty list is honoured as "no known level".
   */
  efforts?: readonly string[] | null;
  /** Model label shown as the menu's subtitle, when the caller knows it. */
  modelName?: string;
  /** One-shot level armed for the next message; `null` = none. */
  nextReasoningEffort?: string | null;
  /** Level pinned to the session; `null`/absent = unpinned. */
  sessionReasoningEffort?: string | null;
  /** Arm (`level`) or clear (`null`) the one-shot. */
  onUseOnce?: (level: string | null) => void;
  /** Pin a level to the session. Absent hides the pin affordance entirely. */
  onPin?: (level: string) => void;
  onUnpin?: () => void;
  /** True while the session PATCH is in flight. */
  pinPending?: boolean;
  /** Error from the last session PATCH, already translated. */
  pinError?: string | null;
}

export function ReasoningEffortMenu({
  efforts,
  modelName,
  nextReasoningEffort = null,
  sessionReasoningEffort = null,
  onUseOnce,
  onPin,
  onUnpin,
  pinPending = false,
  pinError = null,
}: ReasoningEffortMenuProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // The caller already resolved the offer (`lib/reasoningEffort`): the active
  // model's `offered_efforts`, or the same declaration mirrored the way the
  // backend offers it. Re-resolving here would lose the capability flags and
  // put `none` back on a model that cannot be switched off. Only an absent
  // list — no caller data at all — falls back to the canonical vocabulary.
  const levels = useMemo(
    () => (efforts == null ? [...CANONICAL_REASONING_EFFORTS] : [...efforts]),
    [efforts],
  );

  const pinned = sessionReasoningEffort ?? null;
  // What the next message will actually use: the one-shot outranks the pin.
  const effective = nextReasoningEffort ?? pinned;

  // A stored level the model's declared set does not list must still have a
  // row — the operator has to see what is in effect, or Default would claim a
  // turn the pill says is pinned. It is drawn disabled: the model does not
  // declare it, so it is not offered, re-armed, or pinned from here.
  const options = useMemo(
    () => (effective !== null && !levels.includes(effective) ? [...levels, effective] : levels),
    [levels, effective],
  );

  const canPin = typeof onPin === "function";
  const hasPinBanner = canPin && !!pinned;

  const armed = nextReasoningEffort !== null || pinned !== null;
  const pillLabel = effective !== null ? reasoningEffortLabel(effective) : t("chat.reasoning");

  return (
    <div className="relative" ref={rootRef} onKeyDown={(e) => { if (e.key === "Escape") setOpen(false); }}>
      <button
        type="button"
        data-testid="reasoning-effort-button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(v => !v)}
        title={t("chat.reasoning_hint")}
        className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] font-medium transition-colors ${
          armed
            ? "border-brand/40 bg-brand/10 text-brand"
            : "border-border-subtle bg-surface text-text-dim hover:text-text hover:border-border"
        }`}
      >
        <Gauge className="h-3 w-3" aria-hidden="true" />
        <span>{pillLabel}</span>
        {nextReasoningEffort !== null && (
          <span
            data-testid="reasoning-next-badge"
            className="rounded-full bg-brand/15 px-1.5 text-[9px] font-semibold uppercase tracking-wide"
          >
            {t("chat.reasoning_next_badge")}
          </span>
        )}
        {pinned !== null && (
          <Pin data-testid="reasoning-pin-indicator" className="h-3 w-3" aria-hidden="true" />
        )}
        <ChevronDown
          className={`h-2.5 w-2.5 shrink-0 transition-transform ${open ? "rotate-180" : ""}`}
          aria-hidden="true"
        />
      </button>

      {open && (
        <div
          role="menu"
          aria-label={t("chat.reasoning_menu_title")}
          className="absolute bottom-full left-0 mb-1 w-72 bg-surface border border-border-subtle rounded-xl shadow-xl z-50 overflow-hidden"
        >
          {/* Header */}
          <div className="p-2 border-b border-border-subtle/50">
            <div className="flex items-baseline justify-between gap-2 px-1">
              <span className="text-[10px] font-semibold text-text-dim/50 uppercase tracking-wider">
                {t("chat.reasoning_menu_title")}
              </span>
              {modelName && (
                <span className="text-[10px] font-mono text-text-dim/40 truncate max-w-[140px]">
                  {modelName}
                </span>
              )}
            </div>
          </div>

          {/* Session pin banner — the pinned level, and the way out of it. */}
          {hasPinBanner && (
            <div className="flex items-center gap-2 px-2.5 py-2 border-b border-border-subtle/50 bg-brand/5">
              <Pin className="h-3 w-3 text-brand shrink-0" aria-hidden="true" />
              <span className="text-[10px] text-brand flex-1 truncate">
                {t("chat.reasoning_pinned", { level: reasoningEffortLabel(pinned!) })}
              </span>
              <button
                type="button"
                onClick={() => onUnpin?.()}
                disabled={pinPending}
                className="text-[10px] font-medium text-text-dim hover:text-error transition-colors disabled:opacity-40"
              >
                {t("chat.reasoning_unpin")}
              </button>
            </div>
          )}

          <div className={`max-h-64 overflow-y-auto scrollbar-thin p-1.5 space-y-0.5 ${pinPending ? "pointer-events-none opacity-60" : ""}`}>
            {options.map((level) => {
              const checked = effective === level;
              // Offered = declared by the active model (or canonical fallback
              // when it declared nothing). A stored value outside the set is
              // shown, never offered.
              const offered = levels.includes(level);
              return (
                <div key={level} className="flex items-center gap-1">
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={checked}
                    disabled={!offered}
                    title={offered ? t("chat.reasoning_use_hint") : t("chat.reasoning_unsupported_hint")}
                    onClick={() => { onUseOnce?.(level); setOpen(false); }}
                    className={`flex-1 flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-left transition-colors disabled:cursor-not-allowed ${
                      checked
                        ? "bg-brand/10 text-brand"
                        : offered
                          ? "hover:bg-surface-hover text-text-dim"
                          : "text-text-dim/40"
                    }`}
                  >
                    <Check className={`h-3 w-3 shrink-0 ${checked ? "opacity-100" : "opacity-0"}`} aria-hidden="true" />
                    <span className="text-xs font-medium">{reasoningEffortLabel(level)}</span>
                  </button>
                  {canPin && (
                    <button
                      type="button"
                      data-testid={`reasoning-pin-${level}`}
                      aria-pressed={pinned === level}
                      aria-label={t("chat.reasoning_pin_hint", { level: reasoningEffortLabel(level) })}
                      title={offered ? t("chat.reasoning_pin_hint", { level: reasoningEffortLabel(level) }) : t("chat.reasoning_unsupported_hint")}
                      disabled={pinPending || !offered}
                      onClick={() => { onPin?.(level); setOpen(false); }}
                      className={`p-1.5 rounded-lg transition-colors shrink-0 disabled:opacity-40 ${
                        pinned === level
                          ? "text-brand bg-brand/10"
                          : "text-text-dim/40 hover:text-brand hover:bg-surface-hover"
                      }`}
                    >
                      <Pin className="h-3 w-3" aria-hidden="true" />
                    </button>
                  )}
                </div>
              );
            })}

            {/* Default: no per-message override. What still applies is whatever
                the session or the agent manifest declares. */}
            <button
              type="button"
              role="menuitemradio"
              aria-checked={effective === null}
              title={t("chat.reasoning_default_hint")}
              onClick={() => { onUseOnce?.(null); setOpen(false); }}
              className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-left transition-colors ${effective === null ? "bg-brand/10 text-brand" : "hover:bg-surface-hover text-text-dim"}`}
            >
              <Check className={`h-3 w-3 shrink-0 ${effective === null ? "opacity-100" : "opacity-0"}`} aria-hidden="true" />
              <span className="text-xs font-medium">{t("chat.reasoning_default")}</span>
            </button>
          </div>

          <div className="px-2.5 py-2 border-t border-border-subtle/50">
            {pinError && <p className="text-error text-[10px] mb-1.5">{pinError}</p>}
            <p className="text-[10px] leading-relaxed text-text-dim/60">
              {canPin ? t("chat.reasoning_menu_hint") : t("chat.reasoning_menu_hint_no_pin")}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
