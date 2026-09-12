import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle, Loader2, Route } from "../lib/lucide";
import type { AgentDetail, CostTier } from "../api";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { EmptyState } from "./ui/EmptyState";
import { useUIStore } from "../lib/store";
import {
  useAgentModelRouting,
  useModelRouterProfiles,
} from "../lib/queries/modelRouter";
import { useUpdateAgentModelRouting } from "../lib/mutations/modelRouter";

const COST_TIERS: CostTier[] = ["cheap", "medium", "expensive"];

/**
 * Per-agent model routing: fixed model vs router-chosen, plus the profile
 * allowlist and the cost budget that constrain the router's choice.
 *
 * The profile catalog comes from the server (builtin asset merged with
 * `~/.librefang/model_profiles.toml`) rather than being hardcoded here, so an
 * operator's own profiles are pickable without a dashboard rebuild.
 */
export function AgentModelRoutingPanel({ agent }: { agent: AgentDetail }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const agentId = agent.id;
  const routingQuery = useAgentModelRouting(agentId);
  const profilesQuery = useModelRouterProfiles();
  const updateRouting = useUpdateAgentModelRouting();

  const saved = routingQuery.data;
  const [mode, setMode] = useState<"fixed" | "flexible">("fixed");
  const [allowed, setAllowed] = useState<string[]>([]);
  const [budget, setBudget] = useState<CostTier | null>(null);

  // Re-seed the draft whenever the server state changes (initial load, or a
  // save that normalised what was sent). Without this the form would keep
  // showing a stale draft after the response came back.
  useEffect(() => {
    if (!saved) return;
    setMode(saved.mode);
    setAllowed(saved.allowed_profiles ?? []);
    setBudget(saved.cost_budget ?? null);
  }, [saved]);

  const profiles = profilesQuery.data?.profiles ?? [];
  const routerEnabled = profilesQuery.data?.enabled ?? false;

  const isDirty = useMemo(() => {
    if (!saved) return false;
    const savedAllowed = [...(saved.allowed_profiles ?? [])].sort();
    const draftAllowed = [...allowed].sort();
    return (
      saved.mode !== mode ||
      (saved.cost_budget ?? null) !== budget ||
      savedAllowed.join(",") !== draftAllowed.join(",")
    );
  }, [saved, mode, allowed, budget]);

  const toggleProfile = (name: string) => {
    setAllowed((prev) =>
      prev.includes(name) ? prev.filter((p) => p !== name) : [...prev, name],
    );
  };

  const handleSave = () => {
    updateRouting.mutate(
      {
        agentId,
        routing: {
          mode,
          // Fixed mode carries no constraints: they describe a routing
          // decision that will not happen, and the server clears them.
          allowed_profiles: mode === "flexible" ? allowed : [],
          cost_budget: mode === "flexible" ? budget : null,
        },
      },
      {
        onSuccess: () =>
          addToast(
            t("agents.routing.saved", { defaultValue: "Model routing saved" }),
            "success",
          ),
        onError: (err: unknown) =>
          addToast(
            err instanceof Error
              ? err.message
              : t("agents.routing.saveFailed", {
                  defaultValue: "Could not save model routing",
                }),
            "error",
          ),
      },
    );
  };

  if (routingQuery.isLoading || profilesQuery.isLoading) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-muted">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("common.loading", { defaultValue: "Loading..." })}
      </div>
    );
  }

  if (routingQuery.isError) {
    return (
      <EmptyState
        icon={<AlertCircle className="h-5 w-5" />}
        title={t("agents.routing.loadFailed", {
          defaultValue: "Could not load model routing",
        })}
      />
    );
  }

  return (
    <div className="space-y-6 p-1">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Route className="h-4 w-4" />
            {t("agents.routing.title", { defaultValue: "Model routing" })}
          </h3>
          <p className="mt-1 text-xs text-muted">
            {t("agents.routing.description", {
              defaultValue:
                "Choose whether this agent always uses its own model, or lets the router pick one per task based on the work's complexity.",
            })}
          </p>
        </div>
        {!routerEnabled && (
          <Badge variant="warning">
            {t("agents.routing.disabledKernelWide", {
              defaultValue: "Router off in config.toml",
            })}
          </Badge>
        )}
      </div>

      {/* Mode */}
      <div className="space-y-2">
        <span className="text-xs font-medium uppercase tracking-wide text-muted">
          {t("agents.routing.mode", { defaultValue: "Mode" })}
        </span>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant={mode === "fixed" ? "primary" : "ghost"}
            onClick={() => setMode("fixed")}
          >
            {t("agents.routing.modeFixed", { defaultValue: "Fixed model" })}
          </Button>
          <Button
            size="sm"
            variant={mode === "flexible" ? "primary" : "ghost"}
            onClick={() => setMode("flexible")}
          >
            {t("agents.routing.modeFlexible", {
              defaultValue: "Router chooses",
            })}
          </Button>
        </div>
        <p className="text-xs text-muted">
          {mode === "fixed"
            ? t("agents.routing.modeFixedHint", {
                defaultValue:
                  "This agent always uses the provider and model in its own manifest.",
              })
            : t("agents.routing.modeFlexibleHint", {
                defaultValue:
                  "The router scores each turn and picks a matching profile. Constrain the choice below.",
              })}
        </p>
      </div>

      {mode === "flexible" && (
        <>
          {/* Profile allowlist */}
          <div className="space-y-2">
            <span className="text-xs font-medium uppercase tracking-wide text-muted">
              {t("agents.routing.allowedProfiles", {
                defaultValue: "Allowed profiles",
              })}
            </span>
            {profiles.length === 0 ? (
              <EmptyState
                icon={<Route className="h-5 w-5" />}
                title={t("agents.routing.noProfiles", {
                  defaultValue: "No model profiles configured",
                })}
                description={t("agents.routing.noProfilesHint", {
                  defaultValue:
                    "Add profiles to ~/.librefang/model_profiles.toml to make them selectable here.",
                })}
              />
            ) : (
              <>
                <p className="text-xs text-muted">
                  {allowed.length === 0
                    ? t("agents.routing.anyProfile", {
                        defaultValue:
                          "None selected — the router may pick any profile.",
                      })
                    : t("agents.routing.someProfiles", {
                        count: allowed.length,
                        defaultValue:
                          "The router may only pick from the selected profiles.",
                      })}
                </p>
                <ul className="space-y-1">
                  {profiles.map((profile) => (
                    <li key={profile.name}>
                      <label className="flex cursor-pointer items-center gap-3 rounded-xl border border-border-subtle px-3 py-2 text-sm hover:border-brand">
                        <input
                          type="checkbox"
                          checked={allowed.includes(profile.name)}
                          onChange={() => toggleProfile(profile.name)}
                        />
                        <span className="font-medium">{profile.name}</span>
                        <Badge variant="default">{profile.cost_tier}</Badge>
                        <span className="text-xs text-muted">
                          {profile.provider}/{profile.model}
                        </span>
                        {profile.description && (
                          <span className="ml-auto truncate text-xs text-muted">
                            {profile.description}
                          </span>
                        )}
                      </label>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>

          {/* Cost budget */}
          <div className="space-y-2">
            <span className="text-xs font-medium uppercase tracking-wide text-muted">
              {t("agents.routing.costBudget", { defaultValue: "Cost budget" })}
            </span>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant={budget === null ? "primary" : "ghost"}
                onClick={() => setBudget(null)}
              >
                {t("agents.routing.noCap", { defaultValue: "No cap" })}
              </Button>
              {COST_TIERS.map((tier) => (
                <Button
                  key={tier}
                  size="sm"
                  variant={budget === tier ? "primary" : "ghost"}
                  onClick={() => setBudget(tier)}
                >
                  {tier}
                </Button>
              ))}
            </div>
            <p className="text-xs text-muted">
              {t("agents.routing.costBudgetHint", {
                defaultValue:
                  "The router will never pick a profile above this tier, even if the task matches one.",
              })}
            </p>
          </div>
        </>
      )}

      <div className="flex justify-end">
        <Button
          size="sm"
          onClick={handleSave}
          disabled={!isDirty || updateRouting.isPending}
        >
          {updateRouting.isPending
            ? t("common.saving", { defaultValue: "Saving..." })
            : t("common.save", { defaultValue: "Save" })}
        </Button>
      </div>
    </div>
  );
}
