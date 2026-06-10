import type { Metadata } from "next";

// Per-route metadata must live in a server module under Next 16: an MDX
// page that exports `metadata` while its component graph pulls in a
// "use client" module (the shared mdx-components provider) is rejected.
// Hosting the title here keeps zh/page.mdx free of an inline metadata
// export while rendering the identical <title>.
export const metadata: Metadata = {
	title: "BossFang - Agent Operating System",
};

export default function ZhLayout({
	children,
}: {
	children: React.ReactNode;
}) {
	return children;
}
