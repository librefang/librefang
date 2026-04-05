import type { ReactNode } from "react";

interface InlineEmptyProps {
  icon?: ReactNode;
  message: string;
  className?: string;
}

/// Lightweight empty-state hint for compact contexts (inside cards,
/// sidebars, table cells, list sections) where the full `EmptyState`
/// component's dashed-border card would be visually heavy.
///
/// Use `EmptyState` for full-section empties; use `InlineEmpty` when
/// the empty message is a small area inside a larger layout.
export function InlineEmpty({ icon, message, className = "" }: InlineEmptyProps) {
  return (
    <div
      className={`flex flex-col items-center justify-center gap-2 py-8 text-text-dim/70 ${className}`}
    >
      {icon && <div className="text-text-dim/40">{icon}</div>}
      <p className="text-xs font-medium">{message}</p>
    </div>
  );
}
