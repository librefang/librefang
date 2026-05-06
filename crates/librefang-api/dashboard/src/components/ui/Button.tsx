import { forwardRef, memo, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Loader2 } from "lucide-react";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "success";
export type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  leftIcon?: ReactNode;
  rightIcon?: ReactNode;
  isLoading?: boolean;
}

const variantStyles: Record<ButtonVariant, string> = {
  primary:
    "bg-linear-to-b from-sky-400/95 to-sky-500/95 text-slate-900 border border-sky-400/60 " +
    "shadow-[0_0_0_1px_rgba(56,189,248,0.18),0_4px_14px_-6px_rgba(56,189,248,0.55),inset_0_1px_0_rgba(255,255,255,0.25)] " +
    "hover:brightness-105",
  secondary:
    "border border-border-subtle bg-surface text-text-main shadow-sm " +
    "hover:bg-main/60 hover:border-brand/30",
  ghost: "bg-transparent text-text-dim hover:text-text-main hover:bg-main/40",
  danger:
    "bg-error/90 text-white border border-error/50 hover:brightness-110 " +
    "shadow-[0_4px_14px_-6px_rgba(220,38,38,0.45)]",
  success:
    "bg-success/90 text-white border border-success/50 hover:brightness-110 " +
    "shadow-[0_4px_14px_-6px_rgba(22,163,74,0.45)]",
};

const sizeStyles: Record<ButtonSize, string> = {
  sm: "px-2.5 h-7 text-xs",
  md: "px-3 h-8 text-[13px]",
  lg: "px-4 h-9 text-sm",
};

export const Button = memo(forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      className = "",
      variant = "primary",
      size = "md",
      leftIcon,
      rightIcon,
      isLoading,
      disabled,
      children,
      ...props
    },
    ref
  ) => {
    return (
      <button
        ref={ref}
        disabled={disabled || isLoading}
        aria-busy={isLoading}
        className={`
          inline-flex items-center justify-center gap-1.5 rounded-lg font-semibold
          tracking-tight whitespace-nowrap
          transition-[background,border-color,box-shadow,filter] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]
          active:scale-[0.97] active:duration-100
          focus:outline-none focus:ring-2 focus:ring-brand/30 focus:ring-offset-1
          disabled:opacity-50 disabled:cursor-not-allowed disabled:active:scale-100
          ${variantStyles[variant]}
          ${sizeStyles[size]}
          ${className}
        `}
        {...props}
      >
        {isLoading ? (
          <Loader2
            className={`animate-spin ${
              size === "sm" ? "h-3 w-3" : size === "lg" ? "h-5 w-5" : "h-4 w-4"
            }`}
          />
        ) : (
          leftIcon
        )}
        {children}
        {!isLoading && rightIcon}
      </button>
    );
  }
));

Button.displayName = "Button";
