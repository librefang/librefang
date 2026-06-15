import type { SVGProps } from "react";

// Server-component inline SVG mirroring lucide-react's `Bot` icon
// (v1.17.0). lucide's React icons import a "use client" context module,
// which under Next 16 poisons any MDX page that also exports `metadata`
// (metadata must resolve on a server component). Rendering the same
// paths inline keeps the icon a server component so home/zh MDX pages
// stay server-rendered with identical output.
export function Bot(props: SVGProps<SVGSVGElement>) {
	return (
		<svg
			xmlns="http://www.w3.org/2000/svg"
			width={24}
			height={24}
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth={2}
			strokeLinecap="round"
			strokeLinejoin="round"
			aria-hidden="true"
			{...props}
		>
			<path d="M12 8V4H8" />
			<rect width="16" height="12" x="4" y="8" rx="2" />
			<path d="M2 14h2" />
			<path d="M20 14h2" />
			<path d="M15 13v2" />
			<path d="M9 13v2" />
		</svg>
	);
}
