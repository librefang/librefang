import { slugifyWithCounter } from "@sindresorhus/slugify";
import * as acorn from "acorn";
import { toString } from "mdast-util-to-string";
import { mdxAnnotations } from "mdx-annotations";
import { createHighlighter } from "shiki";
import { visit } from "unist-util-visit";
import { fromHtml } from "hast-util-from-html";

function rehypeParseCodeBlocks() {
	return function(tree) {
		visit(tree, "element", function(node, _nodeIndex, parentNode) {
			if (node.tagName === "code") {
				parentNode.properties.language = node.properties.className
					? node.properties.className[0].replace(/^language-/, "")
					: "bash";
			}
		});
	};
}

let highlighter;

function rehypeShiki() {
	return async function(tree) {
		highlighter =
			highlighter ??
			(await createHighlighter({
				themes: ["github-light"],
				langs: [
					"javascript", "bash", "json", "typescript", "shell", "tsx", "jsx",
					"python", "php", "ruby", "go", "rust", "java", "c", "cpp",
					"csharp", "html", "css", "sql", "yaml", "xml", "markdown", "toml",
					"nginx", "dockerfile", "ini", "powershell", "docker"
				],
			}));

		visit(tree, "element", function(node) {
			if (node.tagName === "pre" && node.children[0] && node.children[0].tagName === "code") {
				const codeNode = node.children[0];
				const textNode = codeNode.children[0];

				if (!textNode || textNode.type !== 'text') return;

				node.properties.code = textNode.value;

				if (node.properties.language === 'mermaid') {
					const encodedCode = Buffer.from(textNode.value).toString('base64');
					node.tagName = 'img';
					node.properties = {
						src: "https://mermaid.ink/img/" + encodedCode,
						alt: 'Mermaid Diagram',
						className: 'mermaid-diagram',
						style: 'max-width: 100%; height: auto; margin: 2rem 0; display: block;'
					};
					node.children = [];
					return;
				}

				if (node.properties.language) {
					const html = highlighter.codeToHtml(textNode.value, {
						lang: node.properties.language,
						theme: "github-light",
					});
					
					const hast = fromHtml(html, { fragment: true });
					const preNode = hast.children[0];
					if (preNode && preNode.tagName === 'pre') {
						const innerCodeNode = preNode.children[0];
						if (innerCodeNode && innerCodeNode.tagName === 'code') {
							codeNode.children = innerCodeNode.children;
							codeNode.properties = Object.assign({}, codeNode.properties, innerCodeNode.properties);
							node.properties = Object.assign({}, node.properties, preNode.properties);
						}
					}
				}
			}
		});
	};
}

function rehypeSlugify() {
	return function(tree) {
		const slugify = slugifyWithCounter();
		visit(tree, "element", function(node) {
			if (node.tagName === "h2" && !node.properties.id) {
				node.properties.id = slugify(toString(node));
			}
		});
	};
}

function rehypeAddMDXExports(getExports) {
	return function(tree) {
		const exports = Object.entries(getExports(tree));

		for (var i = 0; i < exports.length; i++) {
			var entry = exports[i];
			var name = entry[0];
			var value = entry[1];
			
			var found = false;
			for (var j = 0; j < tree.children.length; j++) {
				var node = tree.children[j];
				if (
					node.type === "mdxjsEsm" &&
					new RegExp("export\\s+const\\s+" + name + "\\s*=").test(node.value)
				) {
					found = true;
					break;
				}
			}
			
			if (found) continue;

			const exportStr = "export const " + name + " = " + value;

			tree.children.push({
				type: "mdxjsEsm",
				value: exportStr,
				data: {
					estree: acorn.parse(exportStr, {
						sourceType: "module",
						ecmaVersion: "latest",
					}),
				},
			});
		}
	};
}

function getSections(node) {
	const sections = [];
	const children = node.children || [];

	for (var i = 0; i < children.length; i++) {
		var child = children[i];
		if (child.type === "element" && child.tagName === "h2") {
			sections.push("{ title: " + JSON.stringify(toString(child)) + ", id: " + JSON.stringify(child.properties.id) + " }");
		} else if (child.children) {
			const subSections = getSections(child);
			for (var j = 0; j < subSections.length; j++) {
				sections.push(subSections[j]);
			}
		}
	}

	return sections;
}

export const rehypePlugins = [
	mdxAnnotations.rehype,
	rehypeParseCodeBlocks,
	rehypeShiki,
	rehypeSlugify,
	[
		rehypeAddMDXExports,
		function(tree) {
			return {
				sections: "[" + getSections(tree).join(",") + "]"
			};
		}
	],
];
