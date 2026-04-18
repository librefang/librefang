import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { ProviderItem } from "../api";
import { isProviderAvailable } from "../lib/status";
import { useProviders } from "../lib/queries/providers";
import {
  useSetProviderKey,
  useTestProvider,
  useSetDefaultProvider,
} from "../lib/mutations/providers";
import { useQuickInit } from "../lib/mutations/overview";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Zap, Key, Rocket, CheckCircle2, ArrowRight, ArrowLeft, Loader2 } from "lucide-react";

type Step = 1 | 2 | 3;

export function WizardPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const addToast = useUIStore((s) => s.addToast);

  const [step, setStep] = useState<Step>(1);
  const [providerId, setProviderId] = useState<string>("");
  const [apiKey, setApiKey] = useState<string>("");
  const [confirmReplace, setConfirmReplace] = useState(false);
  const [finalizing, setFinalizing] = useState(false);
  const [done, setDone] = useState(false);

  const providersQuery = useProviders();
  const setProviderKeyMutation = useSetProviderKey();
  const testProviderMutation = useTestProvider();
  const setDefaultProviderMutation = useSetDefaultProvider();
  const quickInitMutation = useQuickInit();

  // Recommended providers for first-time setup — prioritize free/fast options.
  const providerOptions = useMemo(() => {
    const all = providersQuery.data ?? [];
    const preferredOrder = ["groq", "openai", "anthropic", "google", "deepseek", "mistral", "ollama"];
    const rank = (p: ProviderItem) => {
      const idx = preferredOrder.indexOf(p.id.toLowerCase());
      return idx === -1 ? preferredOrder.length : idx;
    };
    return [...all].sort((a, b) => rank(a) - rank(b));
  }, [providersQuery.data]);

  const selectedProvider = providerOptions.find((p) => p.id === providerId);
  const requiresKey = selectedProvider?.key_required !== false;
  // If the provider already has a working key and the user is typing a new one,
  // they're about to overwrite a credential we know was good. A failed test
  // post-write leaves the provider broken with no way to restore the old key,
  // so gate the persist behind an explicit confirm checkbox.
  const existingKeyWorking =
    !!selectedProvider && isProviderAvailable(selectedProvider.auth_status);
  const typingNewKey = requiresKey && apiKey.trim().length > 0;
  const needsReplaceConfirm = existingKeyWorking && typingNewKey && !confirmReplace;

  const setKeyMutation = useMutation({
    mutationFn: async () => {
      if (!providerId) throw new Error("no_provider");
      if (requiresKey && apiKey.trim()) {
        await setProviderKeyMutation.mutateAsync({ id: providerId, key: apiKey.trim() });
      }
      const test = await testProviderMutation.mutateAsync(providerId);
      if (test.status !== "ok" && test.status !== "success") {
        throw new Error(test.message || "test_failed");
      }
    },
    onSuccess: () => {
      addToast(t("wizard.provider_connected"), "success");
      setStep(3);
    },
    onError: (err: Error) => {
      addToast(t("wizard.provider_failed", { message: err.message || "" }), "error");
    },
  });

  const finalize = async () => {
    if (!providerId) return;
    setFinalizing(true);
    try {
      await setDefaultProviderMutation.mutateAsync({ id: providerId });
      await quickInitMutation.mutateAsync();
      setDone(true);
    } catch (err) {
      addToast(t("wizard.finalize_failed", { message: (err as Error).message }), "error");
    } finally {
      setFinalizing(false);
    }
  };

  const containerClass = "max-w-2xl mx-auto py-12 px-6 transition-colors duration-300";

  if (done) {
    return (
      <div className={containerClass}>
        <Card padding="lg" className="rounded-3xl text-center">
          <div className="h-16 w-16 mx-auto rounded-3xl bg-success flex items-center justify-center text-white shadow-2xl shadow-success/30 mb-6">
            <CheckCircle2 className="h-10 w-10" />
          </div>
          <h1 className="text-3xl font-black mb-3">{t("wizard.done_title")}</h1>
          <p className="text-text-dim mb-8 max-w-sm mx-auto">{t("wizard.done_desc")}</p>
          <div className="flex justify-center gap-3">
            <Button variant="primary" rightIcon={<ArrowRight className="h-4 w-4" />} onClick={() => navigate({ to: "/" })}>
              {t("wizard.go_overview")}
            </Button>
            <Button variant="secondary" onClick={() => navigate({ to: "/chat", search: { agentId: undefined } })}>
              {t("wizard.start_chat")}
            </Button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className={containerClass}>
      <div className="flex flex-col items-center mb-12">
        <div className="h-16 w-16 rounded-3xl bg-brand flex items-center justify-center text-white shadow-2xl shadow-brand/40 mb-6">
          <Zap className="h-10 w-10" />
        </div>
        <h1 className="text-4xl font-black tracking-tight mb-2">{t("wizard.welcome")}</h1>
        <p className="text-text-dim font-medium text-center max-w-md">{t("wizard.subtitle")}</p>
      </div>

      <Card padding="lg" className="rounded-3xl">
        <div className="flex justify-between items-center mb-8">
          {[1, 2, 3].map((s) => (
            <div key={s} className="flex items-center gap-2 flex-1 last:flex-none">
              <div className={`h-8 w-8 rounded-full flex items-center justify-center text-xs font-black transition-colors ${step >= s ? "bg-brand text-white" : "bg-main text-text-dim border border-border-subtle"}`}>
                {step > s ? <CheckCircle2 className="h-4 w-4" /> : s}
              </div>
              {s < 3 && <div className={`h-1 flex-1 rounded-full ${step > s ? "bg-brand" : "bg-border-subtle"}`} />}
            </div>
          ))}
        </div>

        {step === 1 && (
          <div className="animate-in fade-in slide-in-from-bottom-4">
            <h2 className="text-2xl font-black mb-2">{t("wizard.connect_provider")}</h2>
            <p className="text-text-dim text-sm mb-6">{t("wizard.step_1_desc")}</p>

            {providersQuery.isLoading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2 className="h-6 w-6 animate-spin text-brand" />
              </div>
            ) : (
              <div className="space-y-2">
                {providerOptions.map((p) => {
                  const ready = isProviderAvailable(p.auth_status);
                  const isActive = providerId === p.id;
                  return (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => setProviderId(p.id)}
                      className={`w-full flex items-center justify-between rounded-xl border px-4 py-3 text-left transition-colors ${isActive ? "border-brand bg-brand/5" : "border-border-subtle bg-surface hover:border-brand/30"}`}
                    >
                      <div className="flex flex-col">
                        <span className="text-sm font-bold text-text-main">{p.display_name || p.id}</span>
                        <span className="text-[11px] text-text-dim">{p.id}{p.model_count ? ` · ${p.model_count} models` : ""}</span>
                      </div>
                      {ready ? (
                        <Badge variant="success">{t("wizard.provider_ready")}</Badge>
                      ) : p.key_required === false ? (
                        <Badge variant="info">{t("wizard.no_key")}</Badge>
                      ) : (
                        <Badge variant="warning">{t("wizard.needs_key")}</Badge>
                      )}
                    </button>
                  );
                })}
                {providerOptions.length === 0 && (
                  <p className="text-center text-text-dim text-sm py-8">{t("wizard.no_providers")}</p>
                )}
              </div>
            )}
          </div>
        )}

        {step === 2 && (
          <div className="animate-in fade-in slide-in-from-bottom-4">
            <h2 className="text-2xl font-black mb-2">{t("wizard.add_api_key")}</h2>
            <p className="text-text-dim text-sm mb-6">
              {t("wizard.step_2_desc", { provider: selectedProvider?.display_name || providerId })}
            </p>

            {isProviderAvailable(selectedProvider?.auth_status) && (
              <div className="mb-4 rounded-xl border border-success/30 bg-success/5 p-4 flex items-start gap-3">
                <CheckCircle2 className="h-5 w-5 text-success shrink-0 mt-0.5" />
                <div>
                  <p className="text-sm font-bold text-text-main">{t("wizard.already_configured")}</p>
                  <p className="text-xs text-text-dim mt-1">{t("wizard.already_configured_desc")}</p>
                </div>
              </div>
            )}

            {requiresKey ? (
              <Input
                label={t("wizard.api_key_label")}
                type="password"
                placeholder="sk-..."
                leftIcon={<Key className="h-4 w-4" />}
                value={apiKey}
                onChange={(e) => {
                  setApiKey(e.target.value);
                  // Any edit invalidates a prior confirmation — we don't want
                  // a lingering approval from an earlier typed-then-erased key
                  // to cover a different string the user later retypes.
                  setConfirmReplace(false);
                }}
                autoFocus
              />
            ) : (
              <p className="text-sm text-text-dim">{t("wizard.no_key_needed")}</p>
            )}

            {selectedProvider?.api_key_env && (
              <p className="text-[11px] text-text-dim/70 mt-2">
                {t("wizard.env_hint", { env: selectedProvider.api_key_env })}
              </p>
            )}

            {existingKeyWorking && typingNewKey && (
              <label className="mt-4 flex items-start gap-2 rounded-xl border border-warning/30 bg-warning/5 p-3 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  checked={confirmReplace}
                  onChange={(e) => setConfirmReplace(e.target.checked)}
                />
                <span className="text-xs text-text-dim leading-relaxed">
                  {t("wizard.replace_key_warning", {
                    defaultValue:
                      "This provider is already connected with a working key. Submitting will overwrite it — a failed test cannot restore the previous key. Check to acknowledge.",
                  })}
                </span>
              </label>
            )}

            {setKeyMutation.isError && (
              <p className="text-xs text-error mt-3 font-medium">
                {(setKeyMutation.error as Error)?.message}
              </p>
            )}
          </div>
        )}

        {step === 3 && (
          <div className="animate-in fade-in slide-in-from-bottom-4">
            <h2 className="text-2xl font-black mb-2">{t("wizard.finish_title")}</h2>
            <p className="text-text-dim text-sm mb-6">{t("wizard.step_3_desc")}</p>

            <div className="rounded-xl border border-border-subtle bg-surface p-4 mb-6 space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs text-text-dim uppercase tracking-wider font-bold">{t("wizard.summary_provider")}</span>
                <span className="text-sm font-bold text-text-main">{selectedProvider?.display_name || providerId}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs text-text-dim uppercase tracking-wider font-bold">{t("wizard.summary_status")}</span>
                <Badge variant="success">{t("wizard.provider_ready")}</Badge>
              </div>
            </div>

            <p className="text-xs text-text-dim/80 leading-relaxed">{t("wizard.step_3_hint")}</p>
          </div>
        )}

        <div className="mt-12 flex justify-between">
          <Button
            variant="secondary"
            leftIcon={<ArrowLeft className="h-4 w-4" />}
            disabled={step === 1}
            onClick={() => setStep((s) => Math.max(1, s - 1) as Step)}
          >
            {t("common.back")}
          </Button>
          {step === 1 && (
            <Button
              variant="primary"
              rightIcon={<ArrowRight className="h-4 w-4" />}
              disabled={!providerId}
              onClick={() => setStep(2)}
            >
              {t("common.continue", { defaultValue: "Continue" })}
            </Button>
          )}
          {step === 2 && (
            <Button
              variant="primary"
              rightIcon={<ArrowRight className="h-4 w-4" />}
              isLoading={setKeyMutation.isPending}
              disabled={
                (requiresKey && !apiKey.trim() && !isProviderAvailable(selectedProvider?.auth_status))
                || needsReplaceConfirm
              }
              onClick={() => setKeyMutation.mutate()}
            >
              {t("wizard.connect")}
            </Button>
          )}
          {step === 3 && (
            <Button
              variant="primary"
              rightIcon={<Rocket className="h-4 w-4" />}
              isLoading={finalizing}
              onClick={finalize}
            >
              {t("wizard.finish_action")}
            </Button>
          )}
        </div>
      </Card>

      <p className="text-center text-xs text-text-dim/60 mt-6">
        <button type="button" onClick={() => navigate({ to: "/" })} className="hover:text-text-dim transition-colors">
          {t("wizard.skip")}
        </button>
      </p>
    </div>
  );
}
