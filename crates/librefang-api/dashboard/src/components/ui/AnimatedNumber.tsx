import { useEffect, useRef, useState } from "react";

interface AnimatedNumberProps {
  value: number | string;
  duration?: number;
  prefix?: string;
  suffix?: string;
  decimals?: number;
  className?: string;
}

export function AnimatedNumber({ value, duration = 800, prefix = "", suffix = "", decimals = 0, className = "" }: AnimatedNumberProps) {
  const [display, setDisplay] = useState("0");
  const prevRef = useRef(0);
  const rafRef = useRef<number>(undefined);

  useEffect(() => {
    const numValue = typeof value === "string" ? parseFloat(value.replace(/[^0-9.-]/g, "")) : value;
    if (isNaN(numValue)) { setDisplay(String(value)); return; }

    const start = prevRef.current;
    const end = numValue;
    const startTime = performance.now();

    const animate = (now: number) => {
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / duration, 1);
      // easeOutExpo
      const eased = progress === 1 ? 1 : 1 - Math.pow(2, -10 * progress);
      const current = start + (end - start) * eased;
      setDisplay(current.toFixed(decimals));
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(animate);
      } else {
        prevRef.current = end;
      }
    };

    rafRef.current = requestAnimationFrame(animate);
    return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
  }, [value, duration, decimals]);

  return <span className={className}>{prefix}{display}{suffix}</span>;
}
