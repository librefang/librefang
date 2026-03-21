import { useTranslation } from "react-i18next";
import { AlertCircle } from "lucide-react";

interface ErrorStateProps {
  message?: string;
  onRetry?: () => void;
}

export function ErrorState({ message, onRetry }: ErrorStateProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col items-center justify-center py-16 border border-dashed border-error/20 rounded-3xl bg-error/5">
      <div className="h-12 w-12 rounded-2xl bg-error/10 flex items-center justify-center text-error mb-4">
        <AlertCircle className="h-6 w-6" />
      </div>
      <p className="text-sm font-bold text-error">{message || t("common.error")}</p>
      {onRetry && (
        <button
          onClick={onRetry}
          className="mt-4 px-5 py-2 rounded-xl border border-error/20 bg-error/5 text-error text-xs font-black uppercase tracking-widest hover:bg-error/10 transition-all"
        >
          {t("common.refresh")}
        </button>
      )}
    </div>
  );
}
