import type { Metadata } from "next";

// The home page lives in a "(home)" route group (URL stays "/") so its
// title can be declared in this server layout. Under Next 16 an MDX page
// cannot export `metadata` while its graph pulls in the "use client"
// mdx-components provider, so per-page metadata moves to the segment.
export const metadata: Metadata = {
	title: "BossFang - Documentation",
};

export default function HomeLayout({
	children,
}: {
	children: React.ReactNode;
}) {
	return children;
}
