import { forwardRef, type SelectHTMLAttributes } from "react";

interface SelectOption {
  value: string;
  label: string;
}

interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  options: SelectOption[];
  placeholder?: string;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ className = "", label, options, placeholder, ...props }, ref) => {
    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
            {label}
          </label>
        )}
        <select
          ref={ref}
          className={`
            w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5
            text-sm font-medium text-text-main
            focus:border-brand focus:outline-none focus:ring-1 focus:ring-brand/30
            disabled:opacity-50 disabled:cursor-not-allowed
            transition-all duration-200
            ${className}
          `}
          {...props}
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
      </div>
    );
  }
);

Select.displayName = "Select";
