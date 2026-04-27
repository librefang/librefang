import { motion } from "motion/react";

export function Skeleton({ className = "" }: { className?: string }) {
  return (
    <motion.div
      className={`rounded-lg bg-linear-to-r from-main via-surface-hover to-main bg-[length:200%_100%] ${className}`}
      animate={{ backgroundPosition: ["200% 0", "-200% 0"] }}
      transition={{ duration: 1.5, ease: "easeInOut", repeat: Infinity }}
    />
  );
}

export function CardSkeleton() {
  return (
    <div className="rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm overflow-hidden" role="status" aria-busy="true">
      <div className="flex items-center justify-between mb-4">
        <Skeleton className="h-3 w-24" />
        <Skeleton className="h-8 w-8 rounded-lg" />
      </div>
      <Skeleton className="h-8 w-20 mb-3" />
      <Skeleton className="h-1.5 w-full rounded-full" />
    </div>
  );
}

export function ListSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="space-y-3" role="status" aria-busy="true">
      {Array.from({ length: rows }).map((_, i) => (
        <div
          key={i}
          className="rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm"
        >
          <div className="flex items-center gap-4">
            <Skeleton className="h-10 w-10 rounded-xl shrink-0" />
            <div className="flex-1 space-y-2.5">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-3 w-48" />
            </div>
            <Skeleton className="h-6 w-16 rounded-lg" />
          </div>
        </div>
      ))}
    </div>
  );
}

const GRID_COLS: Record<number, string> = {
  1: "lg:grid-cols-1",
  2: "lg:grid-cols-2",
  3: "lg:grid-cols-3",
  4: "lg:grid-cols-4",
  5: "lg:grid-cols-5",
  6: "lg:grid-cols-6",
};

export function GridSkeleton({ cols = 4 }: { cols?: number }) {
  return (
    <div className={`grid gap-4 sm:grid-cols-2 ${GRID_COLS[cols] ?? "lg:grid-cols-4"}`} role="status" aria-busy="true">
      {Array.from({ length: cols }).map((_, i) => (
        <CardSkeleton key={i} />
      ))}
    </div>
  );
}
