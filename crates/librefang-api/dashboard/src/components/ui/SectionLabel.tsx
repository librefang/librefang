import type { ReactNode } from "react";

interface SectionLabelProps {
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}

export function SectionLabel({ children, action, className = "" }: SectionLabelProps) {
  return (
    <div className={`flex items-center justify-between mb-2.5 ${className}`}>
      <div role="heading" aria-level={3} className="text-[10.5px] font-semibold uppercase tracking-[0.08em] text-text-dim">{children}</div>
      {action}
    </div>
  );
}
