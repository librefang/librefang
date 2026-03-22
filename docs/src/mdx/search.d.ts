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

// Declare module types for .mjs files
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
