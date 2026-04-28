/**
 * ResponsiveTable — renders a `<table>` on md+ viewports and stacked cards on
 * smaller screens. Avoids horizontal scrolling on phones without losing data.
 *
 * Usage:
 *   <ResponsiveTable
 *     columns={[{ key: "name", label: "Name" }, ...]}
 *     rows={items}
 *     rowKey={(row) => row.id}
 *     renderCell={(col, row) => row[col.key]}
 *   />
 */

import type { ReactNode } from "react";

export interface ResponsiveTableColumn<T> {
  key: string;
  label: string;
  /** Hide this column in the card view — use for columns that are
   *  redundant (e.g. a "row number" column) when stacked. */
  hideInCard?: boolean;
  /** Optional custom render. Falls back to `(row as any)[col.key]` if omitted. */
  render?: (row: T) => ReactNode;
  /** th className override. */
  thClass?: string;
  /** td className override. */
  tdClass?: string;
}

interface Props<T> {
  columns: ResponsiveTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string | number;
  /** Extra classes for the wrapping div. */
  className?: string;
  /** Message shown when `rows` is empty. */
  empty?: ReactNode;
}

export function ResponsiveTable<T>({
  columns,
  rows,
  rowKey,
  className = "",
  empty,
}: Props<T>) {
  if (rows.length === 0 && empty) {
    return <div className={className}>{empty}</div>;
  }

  const visibleCols = columns.filter((c) => !c.hideInCard);

  return (
    <div className={className}>
      {/* ── Desktop table (md+) ───────────────────────────── */}
      <div className="hidden md:block overflow-x-auto rounded-xl border border-border-subtle">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border-subtle bg-surface-hover/50">
              {columns.map((col) => (
                <th
                  key={col.key}
                  className={
                    col.thClass ??
                    "px-4 py-3 text-left text-xs font-bold uppercase tracking-wider text-text-dim"
                  }
                >
                  {col.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={rowKey(row)}
                className="border-b last:border-0 border-border-subtle hover:bg-surface-hover/30 transition-colors"
              >
                {columns.map((col) => (
                  <td
                    key={col.key}
                    className={col.tdClass ?? "px-4 py-3 text-sm"}
                  >
                    {col.render ? col.render(row) : String((row as Record<string, unknown>)[col.key] ?? "")}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* ── Mobile cards (< md) ───────────────────────────── */}
      <div className="md:hidden flex flex-col gap-2">
        {rows.map((row) => (
          <div
            key={rowKey(row)}
            className="rounded-xl border border-border-subtle bg-surface p-3 text-sm space-y-1.5"
          >
            {visibleCols.map((col) => (
              <div key={col.key} className="flex items-start gap-2 min-w-0">
                <span className="shrink-0 w-24 text-[11px] font-bold uppercase tracking-wider text-text-dim">
                  {col.label}
                </span>
                <span className="flex-1 min-w-0 text-text break-words">
                  {col.render ? col.render(row) : String((row as Record<string, unknown>)[col.key] ?? "—")}
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
