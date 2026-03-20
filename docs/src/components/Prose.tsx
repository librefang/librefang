import clsx from 'clsx';

export function Prose<T extends React.ElementType = 'div'>({
	as,
	className,
	...props
}: Omit<React.ComponentPropsWithoutRef<T>, 'as' | 'className'> & {
	as?: T;
	className?: string;
}) {
	const Component = as ?? 'div';

	return (
		<Component
			className={clsx(
				className,
				'prose dark:prose-invert max-w-none',
				'[html_:where(&>*)]:mx-auto [html_:where(&>*)]:max-w-4xl lg:[html_:where(&>*)]:mx-[calc(50%-min(50%,var(--container-lg)))] lg:[html_:where(&>*)]:max-w-5xl',
				'[&_table]:block [&_table]:overflow-x-auto [&_table]:whitespace-nowrap [&_th]:px-3 [&_td]:px-3',
			)}
			{...props}
		/>
	);
}
