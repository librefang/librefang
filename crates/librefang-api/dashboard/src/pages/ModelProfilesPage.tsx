import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { toastErr, toast } from "../lib/errors";
import { useUIStore } from "../lib/store";
import { api } from "../api";
import { Cpu, Plus, Trash2, Zap, Brain, Search, Layers } from "lucide-react";

interface ModelProfile {
  name: string;
  tags: string[];
  provider: string;
  model: string;
  context_window?: number;
  cost_tier: "cheap" | "medium" | "expensive";
  priority: number;
  max_complexity: number;
  fallback?: string;
  description?: string;
}

interface RouterConfig {
  enabled: boolean;
  evaluator_model?: string;
  default_profile?: string;
  complexity_threshold: number;
}

const TIER_COLORS: Record<string, string> = {
  cheap: "green",
  medium: "amber",
  expensive: "purple",
};

const PROFILE_ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  quick: Zap,
  coder: Cpu,
  architect: Brain,
  researcher: Search,
};

export function ModelProfilesPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [profiles, setProfiles] = useState<ModelProfile[]>([]);
  const [config, setConfig] = useState<RouterConfig>({
    enabled: false,
    complexity_threshold: 0.3,
  });
  const [loading, setLoading] = useState(true);

  const loadData = async () => {
    try {
      setLoading(true);
      const [profRes, cfgRes] = await Promise.all([
        api.get("/api/model-profiles"),
        api.get("/api/model-router/config"),
      ]);
      const pdata = profRes.ok ? await profRes.json() : { profiles: [] };
      const cdata = cfgRes.ok ? await cfgRes.json() : { enabled: false, complexity_threshold: 0.3 };
      setProfiles(pdata.profiles ?? []);
      setConfig(cdata);
    } catch (err) {
      addToast(toastErr(err, "Failed to load model profiles"), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  const toggleRouter = async () => {
    try {
      const updated = { ...config, enabled: !config.enabled };
      await api.put("/api/model-router/config", updated);
      setConfig(updated);
      addToast(
        updated.enabled ? "ModelRouter enabled (alpha)" : "ModelRouter disabled",
        updated.enabled ? "info" : "success",
      );
      loadData();
    } catch (err) {
      addToast(toastErr(err, "Failed to toggle router"), "error");
    }
  };

  return (
    <div className="w-full space-y-8">
      <PageHeader
        badge="Alpha"
        title="Model Profiles"
        subtitle="Dynamic model selection by task complexity. Assign models by tags — the router picks the best one."
        isFetching={loading}
        onRefresh={loadData}
        icon={<Layers className="h-4 w-4" />}
        helpText="Configure model profiles and enable the ModelRouter to automatically select the right model for each task."
      />

      {/* Router toggle */}
      <Card padding="lg" hover>
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-black tracking-tight">ModelRouter</h3>
            <p className="text-xs text-text-dim mt-1">
              {config.enabled
                ? "Active — tasks are routed to the best model based on complexity and tags."
                : "Inactive — agents use their hardcoded model."}
            </p>
          </div>
          <Button
            variant={config.enabled ? "primary" : "secondary"}
            onClick={toggleRouter}
          >
            {config.enabled ? "Enabled (Alpha)" : "Disabled"}
          </Button>
        </div>
        {config.enabled && (
          <div className="mt-4 pt-4 border-t border-border/30 space-y-2 text-xs text-text-dim">
            <span className="font-bold">Default profile: </span>{config.default_profile ?? "(first)"}
            {" · "}
            <span className="font-bold">Evaluator: </span>{config.evaluator_model ?? "heuristics only"}
            {" · "}
            <span className="font-bold">Threshold: </span>{config.complexity_threshold}
          </div>
        )}
      </Card>

      {/* Profile cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4">
        {profiles.map((p) => {
          const Icon = PROFILE_ICONS[p.name] ?? Layers;
          const tierColor = TIER_COLORS[p.cost_tier] ?? "gray";
          return (
            <Card key={p.name} hover padding="lg" className="flex flex-col">
              <div className="flex items-start gap-3 mb-3">
                <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                  <Icon className="w-5 h-5 text-brand" />
                </div>
                <div className="min-w-0">
                  <h4 className="text-sm font-black tracking-tight">{p.name}</h4>
                  <p className="text-[10px] text-text-dim">
                    {p.provider}/{p.model}
                  </p>
                </div>
              </div>
              <div className="flex flex-wrap gap-1 mb-3">
                {p.tags.slice(0, 4).map((tag) => (
                  <Badge key={tag} color="default" textSize="10px" pill>
                    {tag}
                  </Badge>
                ))}
                {p.tags.length > 4 && (
                  <Badge color="default" textSize="10px" pill>
                    +{p.tags.length - 4}
                  </Badge>
                )}
              </div>
              <div className="flex-1" />
              <div className="flex items-center justify-between text-[10px] text-text-dim mt-2 pt-2 border-t border-border/20">
                <span>
                  <span className={`font-bold text-${tierColor}-500`}>
                    {p.cost_tier}
                  </span>{" "}
                  · cx {p.max_complexity}
                </span>
                <span className="font-black">{p.priority}</span>
              </div>
              {p.context_window && (
                <div className="text-[10px] text-text-dim/60 mt-1">
                  {Math.round(p.context_window / 1024)}k context
                </div>
              )}
            </Card>
          );
        })}
      </div>

      {!loading && profiles.length === 0 && (
        <Card padding="lg">
          <p className="text-sm text-text-dim text-center py-8">
            No profiles configured. Add profiles to ~/.librefang/model_profiles.toml and reload.
          </p>
        </Card>
      )}
    </div>
  );
}
