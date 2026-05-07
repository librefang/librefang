import { type HTMLAttributes, memo, useEffect, useState } from "react";

type AvatarSize = "sm" | "md" | "lg" | "xl";

interface AvatarProps extends HTMLAttributes<HTMLDivElement> {
  fallback: string;
  size?: AvatarSize;
  src?: string;
}

const sizeStyles: Record<AvatarSize, string> = {
  sm: "h-8 w-8 text-xs",
  md: "h-10 w-10 text-sm",
  lg: "h-12 w-12 text-base",
  xl: "h-16 w-16 text-lg",
};

function getInitials(name: string): string {
  const initials = name
    .split(" ")
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
      {src && !imgError ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          onError={() => setImgError(true)}
          className="h-full w-full object-cover"
        />
      ) : (
        getInitials(fallback)
      )}
    </div>
  );
});
