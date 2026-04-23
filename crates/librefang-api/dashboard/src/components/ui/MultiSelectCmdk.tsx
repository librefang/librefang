import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Command } from "cmdk";
import { ChevronDown, X } from "lucide-react";
import { useTranslation } from "react-i18next";

interface MultiSelectCmdkProps {
  options: string[];
  value: string[];
  onChange: (next: string[] | ((prev: string[]) => string[])) => void;
  placeholder?: string;
  disabled?: boolean;
}

export function MultiSelectCmdk({
  options,
  value,
  onChange,
  placeholder = "Search…",
  disabled = false,
}: MultiSelectCmdkProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const [openAbove, setOpenAbove] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filteredOptions = useMemo(() => {
    const selected = new Set(value);
    const searchLower = search.toLowerCase();
    return options.filter(
      (o) => !selected.has(o) && o.toLowerCase().includes(searchLower),
    );
  }, [options, value, search]);

  const focusInput = useCallback(() => {
    inputRef.current?.focus();
  }, []);

  const remove = useCallback(
    (item: string) => {
      onChange((prev) => prev.filter((v) => v !== item));
    },
    [onChange],
  );

  const select = useCallback(
    (item: string) => {
      onChange((prev) => [...prev, item]);
      setSearch("");
    },
    [onChange],
  );

  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;

    const updatePlacement = () => {
      const container = containerRef.current;
      const list = listRef.current;
      if (!container || !list) return;

      const rect = container.getBoundingClientRect();
      const viewportHeight = window.innerHeight;
      const listHeight = Math.min(list.scrollHeight, 240);
      const spaceBelow = viewportHeight - rect.bottom;
      const spaceAbove = rect.top;
      setOpenAbove(spaceBelow < listHeight + 12 && spaceAbove > spaceBelow);
    };

    updatePlacement();
    window.addEventListener("resize", updatePlacement);
    return () => window.removeEventListener("resize", updatePlacement);
  }, [isOpen, filteredOptions.length, value.length, search]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        setIsOpen(false);
        e.preventDefault();
      }
      if (e.key === "Backspace" && search === "" && value.length > 0) {
        remove(value[value.length - 1]);
      }
    },
    [search, value, remove],
  );

  const handleInputFocus = useCallback(() => {
    setIsOpen(true);
  }, []);

  return (
    <div ref={containerRef} className="relative">
      <div
        className={`
          flex flex-wrap items-center gap-1 rounded-xl border border-border-subtle
          bg-surface p-1.5 transition-colors duration-200
          hover:border-brand/20
          focus-within:border-brand focus-within:ring-2 focus-within:ring-brand/10
          ${disabled ? "opacity-50 cursor-not-allowed" : "cursor-text"}
        `}
        onClick={focusInput}
      >
        {value.map((item) => (
          <span
            key={item}
            className="flex items-center gap-1 rounded-md bg-brand/10 px-2 py-0.5 text-[11px] font-mono text-brand"
          >
            {item}
            <button
              type="button"
              aria-label={`Remove ${item}`}
              onClick={(e) => {
                e.stopPropagation();
                remove(item);
              }}
              className="rounded-sm p-0.5 transition-colors hover:bg-brand/20"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <Command className="flex flex-1 items-center min-w-[120px]">
          <Command.Input
            ref={inputRef}
            value={search}
            onValueChange={setSearch}
            onFocus={handleInputFocus}
            onKeyDown={handleKeyDown}
            placeholder={value.length === 0 ? placeholder : t("common.add_more", { defaultValue: "Add more…" })}
            disabled={disabled}
            className="flex-1 min-w-[120px] bg-transparent text-xs text-text outline-none placeholder:text-text-dim/40"
          />
          <ChevronDown className="mr-1 h-3.5 w-3.5 shrink-0 text-text-dim/40" />
          {isOpen && (
            <Command.List
              ref={listRef}
              role="listbox"
              aria-multiselectable="true"
              aria-label={t("common.select_options", { defaultValue: "Select options" })}
              className={`absolute left-0 right-0 z-50 max-h-60 overflow-y-auto rounded-xl border border-border-subtle bg-surface shadow-lg ${openAbove ? "bottom-full mb-1" : "top-full mt-1"}`}
              onMouseDown={(e) => e.preventDefault()}
            >
              <Command.Empty className="px-3 py-4 text-center text-xs text-text-dim">
                {t("common.no_results", { defaultValue: "No results" })}
              </Command.Empty>
              {filteredOptions.map((option) => (
                <Command.Item
                  key={option}
                  value={option}
                  role="option"
                  aria-selected={false}
                  onSelect={select}
                  className="flex cursor-pointer items-center gap-2 px-3 py-2 text-xs text-text-dim transition-colors hover:bg-brand/5 data-[selected=true]:bg-brand/10 data-[selected=true]:text-brand"
                >
                  <span className="truncate">{option}</span>
                </Command.Item>
              ))}
            </Command.List>
          )}
        </Command>
      </div>
    </div>
  );
}
