"use client";

import Giscus from "@giscus/react";
import { useTheme } from "next-themes";

export function Comments() {
	const { resolvedTheme } = useTheme();

	return (
		<div className="mt-12 border-t border-zinc-900/5 pt-8 dark:border-white/5">
			<Giscus
				repo="librefang/librefang"
				repoId="R_kgDORkvylw"
				category="General"
				categoryId="DIC_kwDORkvyl84C4NIZ"
				mapping="pathname"
				strict="1"
				reactionsEnabled="1"
				emitMetadata="0"
				inputPosition="top"
				theme={resolvedTheme === "dark" ? "dark_tritanopia" : "light"}
				lang="en"
				loading="lazy"
			/>
		</div>
	);
}
