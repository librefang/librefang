import { useEffect, useRef, useState } from "react";

const NON_NUMERIC_RE = /[^0-9.-]/g;
const easeOutExpo = (p: number) => (p === 1 ? 1 : 1 - Math.pow(2, -10 * p));

interface AnimatedNumberProps {
  value: number | string;
  duration?: number;
  prefix?: string;
  suffix?: string;
  decimals?: number;
  className?: string;
}

function parseValue(value: number | string): number {
  return typeof value === "string" ? parseFloat(value.replace(NON_NUMERIC_RE, "")) : value;
}

export function AnimatedNumber({ value, duration = 800, prefix = "", suffix = "", decimals = 0, className = "" }: AnimatedNumberProps) {
  const [display, setDisplay] = useState(() => {
    const n = parseValue(value);
    return isNaN(n) ? String(value) : n.toFixed(decimals);
  });
  const prevRef = useRef(0);
  const rafRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    const numValue = parseValue(value);
    if (isNaN(numValue)) { setDisplay(String(value)); return; }

    const start = prevRef.current;
    const end = numValue;
    const startTime = performance.now();

    const animate = (now: number) => {
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = easeOutExpo(progress);
      const current = start + (end - start) * eased;
      setDisplay(current.toFixed(decimals));
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(animate);
      } else {
        prevRef.current = end;
      }
    };

    rafRef.current = requestAnimationFrame(animate);
    return () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        const elapsed = performance.now() - startTime;
        const progress = Math.min(elapsed / duration, 1);
        prevRef.current = start + (end - start) * easeOutExpo(progress);
      }
    };
  }, [value, duration, decimals]);

  return <span className={className}>{prefix}{display}{suffix}</span>;
}
