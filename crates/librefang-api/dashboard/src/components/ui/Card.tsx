import { type HTMLAttributes } from "react";

type CardPadding = "none" | "sm" | "md" | "lg";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  padding?: CardPadding;
  hover?: boolean;
  glow?: boolean;
}

const paddingStyles: Record<CardPadding, string> = {
  none: "",
  sm: "p-2.5 sm:p-3",
  md: "p-3 sm:p-4",
  lg: "p-4 sm:p-6",
};

export function Card({
  className = "",
  padding = "md",
  hover = false,
  glow = false,
  children,
  onClick,
  onKeyDown,
  role,
  tabIndex,
  ...props
}: CardProps) {
  // `hover` controls the visual hover effect (border tint + shadow lift).
  // The pointer cursor is gated on actual clickability so we don't
  // mislead users into clicking cards that have nothing wired up
  // (e.g. FangHub skill cards in browse view, plain stat cards).
  const isClickable = typeof onClick === "function";
  return (
    <div
      role={role ?? (isClickable ? "button" : undefined)}
      tabIndex={tabIndex ?? (isClickable ? 0 : undefined)}
      onClick={onClick}
      onKeyDown={
        isClickable
          ? (event) => {
              onKeyDown?.(event);
              if (event.defaultPrevented) return;
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                event.currentTarget.click();
              }
            }
          : onKeyDown
      }
      className={`
        rounded-xl sm:rounded-2xl border border-border-subtle bg-surface shadow-sm
        ${paddingStyles[padding]}
        ${hover ? "hover:border-brand/30 hover:shadow-md transition-shadow" : ""}
        ${isClickable ? "cursor-pointer" : ""}
        ${glow ? "card-glow" : ""}
        ${className}
      `}
      {...props}
    >
      {children}
    </div>
  );
}
