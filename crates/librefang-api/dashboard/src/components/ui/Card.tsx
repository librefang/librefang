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
  ...props
}: CardProps) {
  // `hover` controls the visual hover effect (border tint + shadow lift).
  // The pointer cursor is gated on actual clickability so we don't
  // mislead users into clicking cards that have nothing wired up
  // (e.g. FangHub skill cards in browse view, plain stat cards).
  const isClickable = typeof props.onClick === "function";
  const handleKeyDown = isClickable
    ? (e: React.KeyboardEvent<HTMLDivElement>) => {
        if (e.key === " ") e.preventDefault();
        if (e.key === "Enter" || e.key === " ") {
          props.onClick?.({} as React.MouseEvent<HTMLDivElement>);
        }
      }
    : undefined;
  return (
    <div
      role={isClickable ? "button" : undefined}
      tabIndex={isClickable ? 0 : undefined}
      className={`
        rounded-xl sm:rounded-2xl border border-border-subtle bg-surface shadow-sm
        ${paddingStyles[padding]}
        ${hover ? "hover:border-brand/30 hover:shadow-md transition-[shadow,border-color]" : ""}
        ${isClickable ? "cursor-pointer" : ""}
        ${glow ? "card-glow" : ""}
        ${className}
      `}
      {...props}
      onKeyDown={handleKeyDown}
    >
      {children}
    </div>
  );
}
