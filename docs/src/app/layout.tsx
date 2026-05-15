import glob from "fast-glob";
import type { Metadata } from "next";
import { Providers } from "@/app/providers";
import { Layout } from "@/components/Layout";
import type { Section } from "@/components/SectionProvider";

import "@/styles/tailwind.css";

export const metadata: Metadata = {
	title: {
		template: "%s - BossFang Docs",
		default: "BossFang Docs",
	},
	other: {
		"script-src": "https://librefang-counter.suzukaze-haduki.workers.dev",
	},
};

export default async function RootLayout({
	children,
}: {
	children: React.ReactNode;
}) {
	const pages = await glob("**/*.mdx", { cwd: "src/app" });
	const allSectionsEntries = (await Promise.all(
		pages.map(async (filename) => {
			try {
				const module = await import(`./${filename}`);
				return [
					`/${filename.replace(/(^|\/)page\.mdx$/, "")}`,
					module.sections || [],
				];
			} catch (e) {
				return [`/${filename.replace(/(^|\/)page\.mdx$/, "")}`, []];
			}
		}),
	)) as Array<[string, Array<Section>]>;
	const allSections = Object.fromEntries(allSectionsEntries);

	return (
		<html lang="en" className="h-full" suppressHydrationWarning>
			<head>
				<script
					src="https://librefang-counter.suzukaze-haduki.workers.dev/script.js"
					async
				></script>
			</head>
			<body className="flex min-h-full bg-white antialiased dark:bg-zinc-900">
				<Providers>
					<div className="w-full">
						<Layout allSections={allSections}>{children}</Layout>
					</div>
				</Providers>
			</body>
		</html>
	);
}
