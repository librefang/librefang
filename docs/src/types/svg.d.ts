declare module "*.svg" {
	const content: any;
	export default content;
}

declare module "*.svg?url" {
	const content: string;
	export default content;
}

declare module "*.svg?component" {
	import type { FunctionComponent, SVGProps } from "react";
	const content: FunctionComponent<SVGProps<SVGSVGElement>>;
	export default content;
}
