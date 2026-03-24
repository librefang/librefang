import type { ReactNode } from "react";

interface EmptyStateProps {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
}

export function EmptyState({ icon, title, description, action }: EmptyStateProps) {
  return (
    <div className="col-span-full flex flex-col items-center justify-center py-20 border border-dashed border-border-subtle rounded-3xl bg-gradient-to-b from-surface/50 to-transparent">
      {icon && (
        <div className="relative mb-5">
          <div className="h-16 w-16 rounded-2xl bg-brand/5 flex items-center justify-center text-brand">
            {icon}
          </div>
          <span className="absolute inset-0 rounded-2xl bg-brand/5 animate-pulse" style={{ animationDuration: "3s" }} />
        </div>
      )}
      <h3 className="text-lg font-black tracking-tight">{title}</h3>
      {description && (
        <p className="text-sm text-text-dim mt-2 max-w-sm text-center leading-relaxed">{description}</p>
      )}
      {action && <div className="mt-6">{action}</div>}
    </div>
  );
}
