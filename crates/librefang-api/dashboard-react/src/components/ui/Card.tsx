import { type HTMLAttributes } from "react";

type CardPadding = "none" | "sm" | "md" | "lg";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  padding?: CardPadding;
  hover?: boolean;
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
  children,
  ...props
}: CardProps) {
  return (
    <div
      className={`
        rounded-2xl border border-border-subtle bg-surface shadow-sm
        ${paddingStyles[padding]}
        ${hover ? "hover:border-brand/30 transition-all duration-200 cursor-pointer" : ""}
        ${className}
      `}
      {...props}
    >
      {children}
    </div>
  );
}
