import nextMDX from "@next/mdx";

import { recmaPlugins } from "./src/mdx/recma.mjs";
import { rehypePlugins } from "./src/mdx/rehype.mjs";
import { remarkPlugins } from "./src/mdx/remark.mjs";
import withSearch from "./src/mdx/search.mjs";

/** @type {import('next').NextConfig} */
const nextConfig = {
	basePath: "",
	assetPrefix: "",
	output: "export",
	pageExtensions: ["js", "jsx", "ts", "tsx", "mdx"],
	outputFileTracingIncludes: {
		"/**/*": ["./src/app/**/*.mdx"],
	},
	images: {
		unoptimized: true,
	},
	experimental: {
		mdxRs: false,
	},
	serverExternalPackages: ['shiki'],
};

const withMDX = nextMDX({
	extension: /\.mdx?$/,
	options: {
		remarkPlugins: remarkPlugins,
		rehypePlugins: rehypePlugins,
		recmaPlugins: recmaPlugins,
	},
});

// 顺序至关重要：先应用搜索增强，再包裹 MDX 处理器
const finalConfig = withMDX(withSearch(nextConfig));

export default finalConfig;
