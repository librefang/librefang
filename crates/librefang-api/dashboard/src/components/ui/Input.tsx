import { forwardRef, useId, type InputHTMLAttributes, type ReactNode } from "react";

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  error?: string;
  leftIcon?: ReactNode;
  rightIcon?: ReactNode;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { className = "", label, error, leftIcon, rightIcon, ...props },
  ref,
) {
  const id = useId();
  const errorId = error ? `${id}-error` : undefined;

  return (
    <div className="flex flex-col gap-1.5">
      {label && (
        <label
          htmlFor={id}
          className="text-[10px] font-black uppercase tracking-widest text-text-dim"
        >
          {label}
        </label>
      )}
      <div className="relative group">
        {leftIcon && (
          <div className="absolute left-3.5 top-1/2 -translate-y-1/2 text-text-dim/40 group-focus-within:text-brand transition-colors">
            {leftIcon}
          </div>
        )}
        <input
          ref={ref}
          id={id}
          aria-invalid={error ? true : undefined}
          aria-describedby={errorId}
          className={`
            w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5
            text-sm font-medium text-text-main placeholder:text-text-dim/40
            focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10
            hover:border-brand/20
            disabled:opacity-50 disabled:cursor-not-allowed
            transition-colors duration-200 shadow-sm
            ${error ? "border-red-500" : ""}
            ${leftIcon ? "pl-11" : ""}
            ${rightIcon ? "pr-11" : ""}
            ${className}
          `}
          {...props}
        />
        {rightIcon && (
          <div className="absolute right-3.5 top-1/2 -translate-y-1/2 text-text-dim/40 group-focus-within:text-brand transition-colors">
            {rightIcon}
          </div>
        )}
      </div>
      {error && (
        <p id={errorId} className="text-xs text-red-500" role="alert">
          {error}
        </p>
      )}
    </div>
  );
});
