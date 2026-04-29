import type { ReactNode } from "react";

export type PillKind =
  | "running"
  | "idle"
  | "error"
  | "pending"
  | "scheduled"
  | "paused"
  | "ok"
  | "approved"
  | "denied";

interface PillProps {
  kind?: PillKind;
  children?: ReactNode;
  dot?: boolean;
  size?: "sm" | "md";
  mono?: boolean;
  className?: string;
}

const PALETTE: Record<PillKind, { dot: string; text: string; bg: string; border: string; label: string; pulse: boolean; glow: boolean }> = {
  running:   { dot: "bg-emerald-400", text: "text-emerald-400", bg: "bg-emerald-400/10", border: "border-emerald-400/20", label: "Running",   pulse: true,  glow: true  },
  idle:      { dot: "bg-slate-400",   text: "text-slate-400",   bg: "bg-slate-400/10",   border: "border-slate-400/20",   label: "Idle",      pulse: false, glow: false },
  error:     { dot: "bg-rose-400",    text: "text-rose-400",    bg: "bg-rose-400/10",    border: "border-rose-400/20",    label: "Error",     pulse: false, glow: false },
  pending:   { dot: "bg-amber-400",   text: "text-amber-400",   bg: "bg-amber-400/10",   border: "border-amber-400/20",   label: "Pending",   pulse: true,  glow: false },
  scheduled: { dot: "bg-violet-400",  text: "text-violet-400",  bg: "bg-violet-400/10",  border: "border-violet-400/20",  label: "Scheduled", pulse: false, glow: false },
  paused:    { dot: "bg-slate-400",   text: "text-slate-400",   bg: "bg-slate-400/10",   border: "border-slate-400/20",   label: "Paused",    pulse: false, glow: false },
  ok:        { dot: "bg-emerald-400", text: "text-emerald-400", bg: "bg-emerald-400/10", border: "border-emerald-400/20", label: "OK",        pulse: false, glow: false },
  approved:  { dot: "bg-emerald-400", text: "text-emerald-400", bg: "bg-emerald-400/10", border: "border-emerald-400/20", label: "Approved",  pulse: false, glow: false },
  denied:    { dot: "bg-rose-400",    text: "text-rose-400",    bg: "bg-rose-400/10",    border: "border-rose-400/20",    label: "Denied",    pulse: false, glow: false },
};

const GLOW_SHADOW: Partial<Record<PillKind, string>> = {
  running: "shadow-[0_0_6px_#34d399]",
};

export function Pill({ kind = "running", children, dot = true, size = "md", mono = false, className = "" }: PillProps) {
  const t = PALETTE[kind];
  const sizing = size === "sm" ? "h-[18px] px-1.5 text-[10px]" : "h-[22px] px-2 text-[11px]";
  return (
    <span
      className={`
        inline-flex items-center gap-1.5 rounded-full border whitespace-nowrap font-medium
        ${sizing}
        ${t.text} ${t.bg} ${t.border}
        ${mono ? "font-mono tracking-normal" : "tracking-[0.01em]"}
        ${className}
      `.trim().replace(/\s+/g, " ")}
    >
      {dot ? (
        <span
          className={`
            w-[5px] h-[5px] rounded-full
            ${t.dot}
            ${t.pulse ? "animate-pulse-soft" : ""}
            ${t.glow ? GLOW_SHADOW[kind] ?? "" : ""}
          `.trim().replace(/\s+/g, " ")}
        />
      ) : null}
      {children ?? t.label}
    </span>
  );
}
