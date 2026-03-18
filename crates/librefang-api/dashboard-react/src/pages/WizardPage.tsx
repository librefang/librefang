import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { listProviders, sendAgentMessage, type ProviderItem } from "../api";

const REFRESH_MS = 30000;

type WizardStep = "welcome" | "provider" | "agent" | "channel" | "done";

export function WizardPage() {
  const [step, setStep] = useState<WizardStep>("welcome");
  const [selectedProvider, setSelectedProvider] = useState<string>("");
  const [selectedTemplate, setSelectedTemplate] = useState<string>("");

  const providersQuery = useQuery({
    queryKey: ["providers", "list", "wizard"],
    queryFn: listProviders,
    refetchInterval: REFRESH_MS
  });

  const providers = providersQuery.data ?? [];

  const templates = [
    { id: "coder", name: "Coder", provider: "anthropic", model: "claude-sonnet-4-20250514", profile: "general-purpose" },
    { id: "analyst", name: "Analyst", provider: "anthropic", model: "claude-sonnet-4-20250514", profile: "general-purpose" },
    { id: "writer", name: "Writer", provider: "anthropic", model: "claude-haiku-3-20240307", profile: "general-purpose" }
  ];

  function nextStep() {
    if (step === "welcome") setStep("provider");
    else if (step === "provider") setStep("agent");
    else if (step === "agent") setStep("channel");
    else if (step === "channel") setStep("done");
  }

  function prevStep() {
    if (step === "provider") setStep("welcome");
    else if (step === "agent") setStep("provider");
    else if (step === "channel") setStep("agent");
    else if (step === "done") setStep("channel");
  }

  const configuredProviders = providers.filter((p) => p.reachable);

  return (
    <section className="flex flex-col items-center justify-center py-12">
      <div className="w-full max-w-xl rounded-xl border border-slate-800 bg-slate-900/70 p-8">
        {/* Progress */}
        <div className="mb-8 flex justify-center gap-2">
          {["welcome", "provider", "agent", "channel", "done"].map((s, i) => (
            <div
              key={s}
              className={`h-2 w-8 rounded-full transition ${
                ["welcome", "provider", "agent", "channel", "done"].indexOf(step) >= i
                  ? "bg-sky-500"
                  : "bg-slate-700"
              }`}
            />
          ))}
        </div>

        {/* Welcome Step */}
        {step === "welcome" && (
          <>
            <h1 className="mb-4 text-center text-2xl font-bold text-sky-400">Welcome to LibreFang</h1>
            <p className="mb-6 text-center text-sm text-slate-400">
              This wizard will help you set up your AI agent operating system in just a few minutes.
            </p>
            <div className="mb-6 rounded-lg border border-slate-700 bg-slate-950/50 p-4">
              <p className="mb-2 text-sm font-semibold text-slate-300">This wizard will help you:</p>
              <ul className="list-disc pl-4 text-xs text-slate-400">
                <li>Connect an LLM provider (Anthropic, OpenAI, etc.)</li>
                <li>Create your first AI agent from templates</li>
                <li>Optionally connect a messaging channel</li>
              </ul>
            </div>
            <div className="flex justify-center gap-3">
              <button
                onClick={nextStep}
                className="rounded-lg border border-sky-500 bg-sky-600 px-6 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
              >
                Get Started
              </button>
            </div>
          </>
        )}

        {/* Provider Step */}
        {step === "provider" && (
          <>
            <h2 className="mb-4 text-center text-xl font-semibold">Connect an LLM Provider</h2>
            <p className="mb-4 text-center text-sm text-slate-400">
              You need at least one LLM provider to power your agents.
            </p>

            {configuredProviders.length > 0 && (
              <div className="mb-4 rounded-lg border border-emerald-700 bg-emerald-700/15 p-3 text-center text-sm text-emerald-400">
                {configuredProviders.length} provider(s) already configured!
              </div>
            )}

            <div className="mb-6 grid gap-2">
              {providers.map((provider) => (
                <button
                  key={provider.id}
                  onClick={() => setSelectedProvider(provider.id)}
                  className={`rounded-lg border p-3 text-left transition ${
                    selectedProvider === provider.id
                      ? "border-sky-500 bg-sky-500/15"
                      : "border-slate-700 bg-slate-950/50 hover:border-slate-600"
                  }`}
                >
                  <p className="text-sm font-medium">{provider.display_name ?? provider.id}</p>
                  <p className="text-xs text-slate-400">
                    {provider.model_count ?? 0} models · {provider.reachable ? "Ready" : "Not configured"}
                  </p>
                </button>
              ))}
            </div>

            <div className="flex justify-center gap-3">
              <button
                onClick={prevStep}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Back
              </button>
              <button
                onClick={nextStep}
                className="rounded-lg border border-sky-500 bg-sky-600 px-6 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
              >
                Continue
              </button>
            </div>
          </>
        )}

        {/* Agent Step */}
        {step === "agent" && (
          <>
            <h2 className="mb-4 text-center text-xl font-semibold">Create Your First Agent</h2>
            <p className="mb-4 text-center text-sm text-slate-400">Pick a template to get started quickly.</p>

            <div className="mb-6 grid gap-2">
              {templates.map((template) => (
                <button
                  key={template.id}
                  onClick={() => setSelectedTemplate(template.id)}
                  className={`rounded-lg border p-3 text-left transition ${
                    selectedTemplate === template.id
                      ? "border-sky-500 bg-sky-500/15"
                      : "border-slate-700 bg-slate-950/50 hover:border-slate-600"
                  }`}
                >
                  <p className="text-sm font-medium">{template.name}</p>
                  <p className="text-xs text-slate-400">
                    {template.provider} / {template.model}
                  </p>
                </button>
              ))}
            </div>

            <div className="flex justify-center gap-3">
              <button
                onClick={prevStep}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Back
              </button>
              <button
                onClick={nextStep}
                className="rounded-lg border border-sky-500 bg-sky-600 px-6 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
              >
                Continue
              </button>
            </div>
          </>
        )}

        {/* Channel Step */}
        {step === "channel" && (
          <>
            <h2 className="mb-4 text-center text-xl font-semibold">Connect a Channel</h2>
            <p className="mb-4 text-center text-sm text-slate-400">
              Channels let your agent communicate via messaging platforms. This is optional.
            </p>

            <div className="mb-6 rounded-lg border border-slate-700 bg-slate-950/50 p-4 text-center">
              <p className="text-sm text-slate-400">
                You can skip this step. The built-in web chat is always available from the Agents page.
              </p>
            </div>

            <div className="flex justify-center gap-3">
              <button
                onClick={prevStep}
                className="rounded-lg border border-slate-600 bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Back
              </button>
              <button
                onClick={nextStep}
                className="rounded-lg border border-sky-500 bg-sky-600 px-6 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
              >
                Skip
              </button>
            </div>
          </>
        )}

        {/* Done Step */}
        {step === "done" && (
          <>
            <h1 className="mb-4 text-center text-2xl font-bold text-emerald-400">You're All Set!</h1>
            <p className="mb-6 text-center text-sm text-slate-400">
              LibreFang is configured and ready to go. Start by creating agents and chatting with them.
            </p>

            <div className="mb-6 space-y-2 rounded-lg border border-slate-700 bg-slate-950/50 p-4">
              <div className="flex justify-between text-sm">
                <span className="text-slate-400">LLM Provider</span>
                <span className="text-emerald-400">
                  {selectedProvider || (configuredProviders.length > 0 ? "Pre-configured" : "Skipped")}
                </span>
              </div>
              <div className="flex justify-between text-sm">
                <span className="text-slate-400">First Agent</span>
                <span className="text-emerald-400">{selectedTemplate || "Skipped"}</span>
              </div>
            </div>

            <div className="flex justify-center gap-3">
              <a
                href="#/agents"
                className="rounded-lg border border-sky-500 bg-sky-600 px-6 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
              >
                Go to Agents
              </a>
              <a
                href="#/chat"
                className="rounded-lg border border-slate-600 bg-slate-800 px-6 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
              >
                Start Chatting
              </a>
            </div>
          </>
        )}
      </div>
    </section>
  );
}
