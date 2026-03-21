export interface Result {
	url: string;
	title: string;
	pageTitle?: string;
	[key: string]: unknown;
}

export interface SearchOptions {
	limit?: number;
}

export function search(query: string, options?: SearchOptions): Result[];

// 为.mjs文件声明模块类型
declare module "@/mdx/search.mjs" {
	export interface Result {
		url: string;
		title: string;
		pageTitle?: string;
		[key: string]: unknown;
	}

	export interface SearchOptions {
		limit?: number;
	}

	export function search(query: string, options?: SearchOptions): Result[];
}
