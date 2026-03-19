import type { ReactNode } from "react";

interface EmptyStateProps {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
}

export function EmptyState({ icon, title, description, action }: EmptyStateProps) {
  return (
    <div className="col-span-full flex flex-col items-center justify-center py-20 border border-dashed border-border-subtle rounded-3xl bg-surface/30">
      {icon && (
        <div className="h-14 w-14 rounded-2xl bg-brand/5 flex items-center justify-center text-brand mb-4">
          {icon}
        </div>
      )}
      <h3 className="text-base font-black tracking-tight">{title}</h3>
      {description && (
        <p className="text-sm text-text-dim mt-1 max-w-xs text-center font-medium">{description}</p>
      )}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
