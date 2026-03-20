import { CloseButton } from '@headlessui/react';
import clsx from 'clsx';
import { motion, useScroll, useTransform } from 'motion/react';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { forwardRef } from 'react';
import { Button } from '@/components/Button';
import { Logo } from '@/components/Logo';
import { MobileNavigation, useIsInsideMobileNavigation, useMobileNavigationStore } from '@/components/MobileNavigation';
import { MobileSearch, Search } from '@/components/Search';
import { ThemeToggle } from '@/components/ThemeToggle';
import { withPrefix } from '@/lib/utils';

function TopLevelNavItem({ href, children }: { href: string; children: React.ReactNode }) {
	return (
		<li>
			<Link
				href={href}
				className='transition text-sm/5 text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-white'
			>
				{children}
			</Link>
		</li>
	);
}

function LangSwitch() {
	const pathname = usePathname();
	const isZh = pathname?.startsWith('/zh');

	// Simple toggle: go to home of other language
	// Default is English (/), Chinese is /zh
	const targetPath = isZh ? '/' : '/zh';

	return (
		<Link
			href={targetPath}
			className="flex items-center gap-1 rounded-full bg-zinc-100 px-2.5 py-1.5 text-xs font-medium text-zinc-600 transition hover:bg-zinc-200 dark:bg-zinc-800 dark:text-zinc-400 dark:hover:bg-zinc-700"
		>
			{isZh ? 'EN' : '中文'}
		</Link>
	);
}

export const Header = forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<typeof motion.div>>(function Header(
	{ className, ...props },
	ref
) {
	const { isOpen: mobileNavIsOpen } = useMobileNavigationStore();
	const isInsideMobileNavigation = useIsInsideMobileNavigation();
	const pathname = usePathname();
	const isZh = pathname?.startsWith('/zh');

	const { scrollY } = useScroll();
	const bgOpacityLight = useTransform(scrollY, [0, 72], ['50%', '90%']);
	const bgOpacityDark = useTransform(scrollY, [0, 72], ['20%', '80%']);

	return (
		<motion.div
			{...props}
			ref={ref}
			className={clsx(
				className,
				'fixed inset-x-0 top-0 z-50 flex h-14 items-center justify-between gap-12 px-4 transition sm:px-6 lg:left-72 lg:z-30 lg:px-8 xl:left-80',
				!isInsideMobileNavigation && 'backdrop-blur-xs lg:left-72 xl:left-80 dark:backdrop-blur-sm',
				isInsideMobileNavigation
					? 'bg-white dark:bg-zinc-900'
					: 'bg-white/(--bg-opacity-light) dark:bg-zinc-900/(--bg-opacity-dark)'
			)}
			style={
				{
					'--bg-opacity-light': bgOpacityLight,
					'--bg-opacity-dark': bgOpacityDark,
				} as React.CSSProperties
			}
		>
			<div
				className={clsx(
					'absolute inset-x-0 top-full h-px transition',
					(isInsideMobileNavigation || !mobileNavIsOpen) && 'bg-zinc-900/7.5 dark:bg-white/7.5'
				)}
			/>
			<Search />
			<div className='flex items-center gap-5 lg:hidden'>
				<MobileNavigation />
				<CloseButton as={Link} href={withPrefix('/')} aria-label='Home'>
					<Logo />
				</CloseButton>
			</div>
			<div className='flex items-center gap-5'>
				<nav className='hidden md:block'>
					<ul className='flex items-center gap-8'>
						<TopLevelNavItem href={withPrefix('/')}>{isZh ? '文档' : 'Docs'}</TopLevelNavItem>
						<TopLevelNavItem href='https://github.com/librefang/librefang'>GitHub</TopLevelNavItem>
					</ul>
				</nav>
				<div className='hidden md:block md:h-5 md:w-px md:bg-zinc-900/10 md:dark:bg-white/15' />
				<div className='flex gap-4'>
					<MobileSearch />
					<LangSwitch />
					<ThemeToggle />
				</div>
			</div>
		</motion.div>
	);
});
