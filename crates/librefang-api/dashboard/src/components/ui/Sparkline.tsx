import { useId } from "react";

interface SparklineProps {
  data: readonly number[];
  width?: number;
  height?: number;
  color?: string;
  filled?: boolean;
  glow?: boolean;
  className?: string;
}

export function Sparkline({
  data,
  width = 220,
  height = 56,
  color = "#38bdf8",
  filled = true,
  glow = true,
  className = "",
}: SparklineProps) {
  const uid = useId().replace(/[^a-zA-Z0-9]/g, "");
  if (!data || data.length < 2) return null;
  const max = Math.max(...data);
  const min = Math.min(...data);
  const range = max - min || 1;
  const stepX = width / (data.length - 1);
  const pts = data.map((v, i) => {
    const x = i * stepX;
    const y = height - 4 - ((v - min) / range) * (height - 8);
    return [x, y] as const;
  });
  const path = pts.map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`)).join(" ");
  const fill = `${path} L${width},${height} L0,${height} Z`;
  return (
    <svg width={width} height={height} className={`block overflow-visible ${className}`} aria-hidden="true">
      <defs>
        <linearGradient id={`sg-${uid}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.32" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
        {glow ? (
          <filter id={`sf-${uid}`} x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="2" />
          </filter>
        ) : null}
      </defs>
      {filled ? <path d={fill} fill={`url(#sg-${uid})`} /> : null}
      {glow ? <path d={path} fill="none" stroke={color} strokeWidth={2} opacity="0.45" filter={`url(#sf-${uid})`} /> : null}
      <path d={path} fill="none" stroke={color} strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
