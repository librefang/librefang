import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

interface SectionLabelProps {
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}

export function SectionLabel({ children, action, className }: SectionLabelProps) {
  return (
    <div className={cn("flex items-center justify-between mb-2.5", className)}>
      <div className="text-label font-semibold uppercase tracking-label text-text-dim">{children}</div>
      {action}
    </div>
  );
}
