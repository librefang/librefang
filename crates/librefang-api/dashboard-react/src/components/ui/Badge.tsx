import { type HTMLAttributes, type ReactNode } from "react";

type BadgeVariant = "default" | "success" | "warning" | "error" | "info" | "brand";

interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant;
  children: ReactNode;
}

const variantStyles: Record<BadgeVariant, string> = {
  default: "bg-main text-text-dim border-border-subtle",
  success: "bg-success/10 text-success border-success/20",
  warning: "bg-warning/10 text-warning border-warning/20",
  error: "bg-error/10 text-error border-error/20",
  info: "bg-info/10 text-info border-info/20",
  brand: "bg-brand/10 text-brand border-brand/20",
};

export function Badge({
  className = "",
  variant = "default",
  children,
  ...props
}: BadgeProps) {
  return (
    <span
      className={`
        inline-flex items-center rounded-lg px-2 py-0.5
        text-[10px] font-black uppercase tracking-wider
        border transition-colors duration-200 whitespace-nowrap
        ${variantStyles[variant]}
        ${className}
      `}
      {...props}
    >
      {children}
    </span>
  );
}
