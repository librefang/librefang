import { type HTMLAttributes } from "react";

type CardPadding = "none" | "sm" | "md" | "lg";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  padding?: CardPadding;
  hover?: boolean;
  glow?: boolean;
}

const paddingStyles: Record<CardPadding, string> = {
  none: "",
  sm: "p-3",
  md: "p-4",
  lg: "p-6",
};

export function Card({
  className = "",
  padding = "md",
  hover = false,
  glow = false,
  children,
  ...props
}: CardProps) {
  return (
    <div
      className={`
        rounded-2xl border border-border-subtle bg-surface shadow-sm
        ${paddingStyles[padding]}
        ${hover ? "hover:border-brand/30 hover:-translate-y-0.5 transition-all duration-300 cursor-pointer card-glow" : "transition-colors duration-200"}
        ${glow ? "card-glow" : ""}
        ${className}
      `}
      {...props}
    >
      {children}
    </div>
  );
}
