import { useEffect } from "react";
import { animate, motion, useMotionValue, useTransform } from "motion/react";

const NUMERIC_STRING_RE = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;

interface AnimatedNumberProps {
  /** Strings animate only when the complete value is a finite decimal or exponent literal. */
  value: number | string;
  /** Animation duration in milliseconds (matches the legacy API). Defaults to 800. */
  duration?: number;
  prefix?: string;
  suffix?: string;
  decimals?: number;
  className?: string;
}

function parseValue(value: number | string): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }

  const normalized = value.trim();
  if (!NUMERIC_STRING_RE.test(normalized)) return null;

  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : null;
}

/// Smoothly tweens a numeric display when `value` changes — used for
/// cost counters, agent counts, latency readouts. Backed by motion's
/// `MotionValue` so the per-frame work happens off the React render
/// path. Falls back to rendering `String(value)` if the input cannot
/// be parsed as a number.
export function AnimatedNumber({
  value,
  duration = 800,
  prefix = "",
  suffix = "",
  decimals = 0,
  className = "",
}: AnimatedNumberProps) {
  const target = parseValue(value);
  const isNumeric = target !== null;
  const motionValue = useMotionValue(target ?? 0);
  const display = useTransform(motionValue, (latest) =>
    `${prefix}${latest.toFixed(decimals)}${suffix}`,
  );

  useEffect(() => {
    if (target === null) return;
    const controls = animate(motionValue, target, {
      duration: duration / 1000,
      ease: [0.25, 0.1, 0.25, 1],
    });
    return () => controls.stop();
  }, [target, duration, motionValue]);

  if (!isNumeric) return <span className={className}>{String(value)}</span>;
  return <motion.span className={className}>{display}</motion.span>;
}
