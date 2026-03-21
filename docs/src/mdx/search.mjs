import { slugifyWithCounter } from "@sindresorhus/slugify";
import glob from "fast-glob";
import * as fs from "fs";
import { toString } from "mdast-util-to-string";
import * as path from "path";
import { remark } from "remark";
import remarkMdx from "remark-mdx";
import { createLoader } from "simple-functional-loader";
import { filter } from "unist-util-filter";
import { SKIP, visit } from "unist-util-visit";
import * as url from "url";

const __filename = url.fileURLToPath(import.meta.url);
const processor = remark().use(remarkMdx).use(extractSections);
const slugify = slugifyWithCounter();

function isObjectExpression(node) {
	return (
		node.type === "mdxTextExpression" &&
		node.data &&
		node.data.estree &&
		node.data.estree.body &&
		node.data.estree.body[0] &&
		node.data.estree.body[0].expression &&
		node.data.estree.body[0].expression.type === "ObjectExpression"
	);
}

function excludeObjectExpressions(tree) {
	return filter(tree, function (node) {
		return !isObjectExpression(node);
	});
}

function extractSections() {
	return function (tree, params) {
		var sections = params.sections;
		slugify.reset();

		visit(tree, function (node) {
			if (node.type === "heading" || node.type === "paragraph") {
				var content = toString(excludeObjectExpressions(node));
				if (node.type === "heading" && node.depth <= 2) {
					var hash = node.depth === 1 ? null : slugify(content);
					sections.push([content, hash, []]);
				} else {
					if (sections.length > 0) {
						sections[sections.length - 1][2].push(content);
					}
				}
				return SKIP;
			}
		});
	};
}

export default function Search(nextConfig) {
	var config = nextConfig || {};
	var cache = new Map();

	return Object.assign({}, config, {
		webpack: function (config, options) {
			config.module.rules.push({
				test: __filename,
				use: [
					createLoader(function () {
						var appDir = path.resolve("./src/app");
						this.addContextDependency(appDir);

						var files = glob.sync("**/*.mdx", { cwd: appDir });
						var data = files.map(function (file) {
							var url = "/" + file.replace(/(^|\/)page\.mdx$/, "");
							var mdx = fs.readFileSync(path.join(appDir, file), "utf8");

							var sections = [];

							if (cache.get(file) && cache.get(file)[0] === mdx) {
								sections = cache.get(file)[1];
							} else {
								var vfile = { value: mdx, sections: sections };
								processor.runSync(processor.parse(vfile), vfile);
								cache.set(file, [mdx, sections]);
							}

							return { url: url, sections: sections };
						});

						return (
							"import FlexSearch from 'flexsearch'\n\nvar sectionIndex = new FlexSearch.Document({\n  tokenize: 'full',\n  document: {\n    id: 'url',\n    index: 'content',\n    store: ['title', 'pageTitle'],\n  },\n  context: {\n    resolution: 9,\n    depth: 2,\n    bidirectional: true\n  }\n})\n\nvar data = " +
							JSON.stringify(data) +
							"\n\nfor (var i = 0; i < data.length; i++) {\n  var item = data[i];\n  var url = item.url;\n  var sections = item.sections;\n  for (var j = 0; j < sections.length; j++) {\n    var section = sections[j];\n    var title = section[0];\n    var hash = section[1];\n    var content = section[2];\n    sectionIndex.add({\n      url: url + (hash ? ('#' + hash) : ''),\n      title: title,\n      content: [title].concat(content).join('\\\\n'),\n      pageTitle: hash ? sections[0][0] : undefined,\n    })\n  }\n}\n\nexport function search(query, options) {\n  var opts = options || {};\n  var searchOptions = Object.assign({}, opts, {\n    enrich: true,\n  });\n  var result = sectionIndex.search(query, searchOptions);\n  if (result.length === 0) {\n    return []\n  }\n  return result[0].result.map(function(item) {\n    return {\n      url: item.id,\n      title: item.doc.title,\n      pageTitle: item.doc.pageTitle,\n    }\n  })\n}"
						);
					}),
				],
			});

			if (typeof config.webpack === "function") {
				return config.webpack(config, options);
			}

			return config;
		},
	});
}
