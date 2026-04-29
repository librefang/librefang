import type { ReactNode } from "react";
import { TrendingUp, TrendingDown } from "lucide-react";
import { Card } from "./Card";

interface KpiProps {
  label: string;
  value: string | number;
  unit?: string;
  delta?: string;
  trend?: "up" | "down" | "flat";
  sub?: string | null;
  sparkline?: ReactNode;
  accent?: boolean;
  onClick?: () => void;
}

export function Kpi({ label, value, unit, delta, trend = "flat", sub, sparkline, accent, onClick }: KpiProps) {
  const trendColor =
    trend === "up" ? "text-emerald-400" : trend === "down" ? "text-rose-400" : "text-text-dim";
  return (
    <Card
      padding="none"
      glow
      onClick={onClick}
      className="relative overflow-hidden surface-lit p-3.5"
    >
      <div className="text-[10.5px] font-semibold uppercase tracking-[0.08em] text-text-dim">{label}</div>
      <div className="flex items-baseline gap-1 mt-2">
        <span
          className={`font-mono font-semibold text-[26px] tracking-[-0.02em] tabular-nums ${
            accent ? "text-brand glow-text" : "text-text-main"
          }`}
        >
          {value}
        </span>
        {unit ? <span className="font-mono text-xs text-text-dim">{unit}</span> : null}
      </div>
      <div className="flex items-center gap-2 mt-1.5">
        {delta ? (
          <span className={`inline-flex items-center gap-0.5 text-[11px] font-mono tabular-nums ${trendColor}`}>
            {trend === "up" ? <TrendingUp className="w-3 h-3" /> : trend === "down" ? <TrendingDown className="w-3 h-3" /> : null}
            {delta}
          </span>
        ) : null}
        {sub ? <span className="text-[11px] text-text-dim">{sub}</span> : null}
      </div>
      {sparkline ? (
        <div className="absolute right-2 bottom-2 opacity-85 pointer-events-none">{sparkline}</div>
      ) : null}
    </Card>
  );
}
