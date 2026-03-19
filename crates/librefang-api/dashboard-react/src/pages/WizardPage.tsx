import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../lib/store";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Zap } from "lucide-react";

export function WizardPage() {
  const { t } = useTranslation();
  const { theme } = useUIStore();
  const [step, setStep] = useState(1);

  const containerClass = "max-w-2xl mx-auto py-12 px-6 transition-colors duration-300";

  return (
    <div className={containerClass}>
      <div className="flex flex-col items-center mb-12">
        <div className="h-16 w-16 rounded-3xl bg-brand flex items-center justify-center text-white shadow-2xl shadow-brand/40 mb-6">
          <Zap className="h-10 w-10" />
        </div>
        <h1 className="text-4xl font-black tracking-tight mb-2">{t("wizard.welcome")}</h1>
        <p className="text-text-dim font-medium text-center">{t("overview.description")}</p>
      </div>

      <Card padding="lg" className="rounded-3xl">
        <div className="flex justify-between items-center mb-8">
          {[1, 2, 3].map((s) => (
            <div key={s} className="flex items-center gap-2">
              <div className={`h-8 w-8 rounded-full flex items-center justify-center text-xs font-black transition-all ${step >= s ? 'bg-brand text-white' : 'bg-main text-text-dim border border-border-subtle'}`}>{s}</div>
              {s < 3 && <div className={`h-1 w-12 rounded-full ${step > s ? 'bg-brand' : 'bg-border-subtle'}`} />}
            </div>
          ))}
        </div>

        {step === 1 && (
          <div className="animate-in fade-in slide-in-from-bottom-4">
            <h2 className="text-2xl font-black mb-2">{t("wizard.connect_provider")}</h2>
            <p className="text-text-dim text-sm mb-8">{t("wizard.step_1")}</p>
            {/* Logic here... */}
          </div>
        )}

        <div className="mt-12 flex justify-between">
          <Button
            variant="secondary"
            disabled={step === 1}
            onClick={() => setStep(s => s - 1)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            onClick={() => step < 3 ? setStep(s => s + 1) : null}
          >
            {t("common.actions")}
          </Button>
        </div>
      </Card>
    </div>
  );
}
