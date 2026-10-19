import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";

/**
 * Free-form list editor: a row of removable chips plus a text box.
 *
 * A value is committed on Enter or when the field loses focus. Commas also
 * commit (and separate pasted values) unless `splitOnComma` is false, which
 * is for fields whose values may contain commas as data, such as regexes or
 * command-line arguments. Semicolons and newlines always separate pasted
 * values. Duplicates are dropped while preserving the order they were
 * entered in; a `value` prop that already carries duplicates is only
 * de-duplicated for rendering, not rewritten.
 */
export function ListField({
  value,
  onChange,
  placeholder,
  disabled = false,
  id,
  ariaLabel,
  splitOnComma = true,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  splitOnComma?: boolean;
}) {
  const { t } = useTranslation();
  const [inputValue, setInputValue] = useState("");

  /** Merge the typed draft and the current value, keeping first occurrences. */
  const mergePending = (pending: string): string[] => {
    const next = [...value];
    for (const item of splitList(pending, splitOnComma)) {
      if (!next.includes(item)) next.push(item);
    }
    return next;
  };

  const commit = (raw: string): void => {
    if (splitList(raw, splitOnComma).length === 0) return;
    setInputValue("");
    const next = mergePending(raw);
    if (next.length !== value.length) onChange(next);
  };

  const remove = (item: string): void => {
    // The blur that precedes a click on the remove button commits the draft
    // as a separate event. Fold that draft into this update so the click
    // cannot overwrite it with a stale `value`.
    const next = mergePending(inputValue).filter((v) => v !== item);
    setInputValue("");
    onChange(next);
  };

  // A `value` carrying duplicates is outside the contract, but rendering it
  // verbatim would produce duplicate React keys. Collapse for display only.
  const items = [...new Set(value)];

  return (
    <div
      className={`flex flex-wrap items-center gap-1.5 rounded-lg border border-border-subtle bg-main px-2 py-1.5 focus-within:border-brand ${
        disabled ? "opacity-60" : ""
      }`}
    >
      {items.map((item) => (
        <span
          key={item}
          className="inline-flex items-center gap-1 rounded-md bg-surface px-1.5 py-0.5 text-[11px] font-medium text-text"
        >
          {item}
          {!disabled && (
            <button
              type="button"
              onClick={() => remove(item)}
              className="text-text-dim hover:text-error"
              aria-label={`${t("common.remove")} ${item}`}
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </span>
      ))}
      <input
        id={id}
        type="text"
        value={inputValue}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => setInputValue(e.target.value)}
        placeholder={items.length === 0 ? placeholder : undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || (splitOnComma && e.key === ",")) {
            e.preventDefault();
            commit(inputValue);
          } else if (e.key === "Backspace" && !inputValue && items.length > 0) {
            const last = items[items.length - 1];
            onChange(value.filter((v) => v !== last));
          }
        }}
        onPaste={(e) => {
          const pasted = e.clipboardData.getData("text");
          if (!hasSeparator(pasted, splitOnComma)) return;
          e.preventDefault();
          commit(inputValue + pasted);
        }}
        onBlur={() => {
          if (inputValue) {
            commit(inputValue);
          }
        }}
        className="flex-1 min-w-[100px] bg-transparent text-xs outline-none placeholder:text-text-dim/40 disabled:cursor-not-allowed"
      />
    </div>
  );
}

/** Whether pasted text contains a separator this field splits on. */
function hasSeparator(raw: string, splitOnComma: boolean): boolean {
  return splitOnComma ? /[,\n;]/.test(raw) : /[\n;]/.test(raw);
}

/** Split a free-form list, dropping blanks. */
function splitList(raw: string, splitOnComma: boolean): string[] {
  return raw
    .split(splitOnComma ? /[,;\n]+/ : /[;\n]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}
