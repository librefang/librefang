import { forwardRef, useId, type SelectHTMLAttributes } from "react";

interface SelectOption {
  value: string;
  label: string;
}

interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  error?: string;
  options: SelectOption[];
  placeholder?: string;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ className = "", label, error, options, placeholder, value, defaultValue, ...props }, ref) => {
    const id = useId();
    const errorId = error ? `${id}-error` : undefined;
    const hasValue = value !== undefined;
    const hasDefault = defaultValue !== undefined;
    const showPlaceholderSelected = !hasValue && !hasDefault;

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={id} className="text-[10px] font-black uppercase tracking-widest text-text-dim">
            {label}
          </label>
        )}
        <select
          id={id}
          ref={ref}
          aria-invalid={error ? true : undefined}
          aria-describedby={errorId}
          className={`
            w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5
            text-sm font-medium text-text-main
            focus:border-brand focus:outline-none focus:ring-1 focus:ring-brand/30
            disabled:opacity-50 disabled:cursor-not-allowed
            transition-colors duration-200
            ${error ? "border-red-500" : ""}
            ${className}
          `}
          {...props}
          {...(hasValue ? { value } : hasDefault ? { defaultValue } : { value: "" })}
        >
          {placeholder && (
            <option value="" disabled>
              {placeholder}
            </option>
          )}
          {options.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
        {error && (
          <p id={errorId} className="text-xs text-red-500" role="alert">
            {error}
          </p>
        )}
      </div>
    );
  }
);

Select.displayName = "Select";
