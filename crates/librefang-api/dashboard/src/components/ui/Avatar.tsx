import { type HTMLAttributes, memo, useEffect, useState } from "react";

type AvatarSize = "sm" | "md" | "lg" | "xl";

interface AvatarProps extends HTMLAttributes<HTMLDivElement> {
  fallback: string;
  size?: AvatarSize;
  src?: string;
  /**
   * Shown instead of the initials when there is no image (#8339).
   *
   * A separate prop rather than something passed through `fallback`, because
   * `fallback` is fed to `getInitials`, which takes `n[0]` — the first UTF-16
   * *code unit*. On an emoji that is a lone surrogate: `"🤖"[0]` is `"\ud83e"`,
   * which renders as the replacement character. `fallback` also stays the
   * `aria-label`, and an emoji is the wrong thing to announce for an agent
   * whose name is right there.
   */
  emoji?: string;
}

const sizeStyles: Record<AvatarSize, string> = {
  sm: "h-8 w-8 text-xs",
  md: "h-10 w-10 text-sm",
  lg: "h-12 w-12 text-base",
  xl: "h-16 w-16 text-lg",
};

function getInitials(name: string): string {
  const initials = name
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((n) => n[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
  return initials || "?";
}

export const Avatar = memo(function Avatar({
  className = "",
  fallback,
  size = "md",
  src,
  emoji,
  ...props
}: AvatarProps) {
  const [imgError, setImgError] = useState(false);
  useEffect(() => setImgError(false), [src]);

  return (
    <div
      role="img"
      aria-label={fallback}
      className={`
        relative flex shrink-0 items-center justify-center
        rounded-full bg-brand/10 text-brand font-black
        overflow-hidden
        ${sizeStyles[size]}
        ${className}
      `}
      {...props}
    >
      {/* Image, then emoji, then initials — the order in which the identity
          was deliberately set. An image that fails to load falls through to
          the emoji for the same reason it falls through to the initials. */}
      {src && !imgError ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          onError={() => setImgError(true)}
          className="h-full w-full object-cover"
        />
      ) : emoji ? (
        // `leading-none` so a tall emoji glyph does not push itself off the
        // circle's vertical centre, and a size bump because an emoji drawn at
        // the initials' font size reads as a speck inside the ring.
        <span className="text-[1.4em] leading-none">{emoji}</span>
      ) : (
        getInitials(fallback)
      )}
    </div>
  );
});
