"use client";

import { usePathname } from "next/navigation";

const REPO_URL = "https://github.com/librefang/librefang";
const DOCS_BASE = "docs/src/app";

export function EditOnGitHub() {
	const pathname = usePathname();
	// Strip trailing slash, map "/" to root page
	const clean = pathname === "/" ? "" : pathname.replace(/\/$/, "");
	const filePath = clean
		? `${DOCS_BASE}${clean}/page.mdx`
		: `${DOCS_BASE}/page.mdx`;
	const editUrl = `${REPO_URL}/edit/main/${filePath}`;

	return (
		<a
			href={editUrl}
			target="_blank"
			rel="noopener noreferrer"
			className="inline-flex items-center gap-1.5 text-sm text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200 transition-colors"
		>
			<svg
				viewBox="0 0 16 16"
				className="h-4 w-4 fill-current"
				aria-hidden="true"
			>
				<path d="M11.013 1.427a1.75 1.75 0 012.474 0l1.086 1.086a1.75 1.75 0 010 2.474l-8.61 8.61c-.21.21-.47.364-.756.445l-3.251.93a.75.75 0 01-.927-.928l.929-3.25c.081-.286.235-.547.445-.758l8.61-8.61zm1.414 1.06a.25.25 0 00-.354 0L3.462 11.1a.25.25 0 00-.064.108l-.631 2.208 2.208-.63a.25.25 0 00.108-.064l8.61-8.61a.25.25 0 000-.354l-1.086-1.086z" />
			</svg>
			Edit this page on GitHub
		</a>
	);
}
