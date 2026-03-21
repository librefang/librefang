"use client";

import { CloseButton } from "@headlessui/react";
import clsx from "clsx";
import { AnimatePresence, motion, useIsPresent } from "motion/react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useRef } from "react";
import { Button } from "@/components/Button";
import { useIsInsideMobileNavigation } from "@/components/MobileNavigation";
import { useSectionStore } from "@/components/SectionProvider";
import { Tag } from "@/components/Tag";
import { remToPx } from "@/lib/remToPx";
import { withPrefix } from "@/lib/utils";

interface NavGroup {
	title: string;
	links: Array<{
		title: string;
		href: string;
	}>;
}

function useInitialValue<T>(value: T, condition = true) {
	const initialValue = useRef(value).current;
	return condition ? initialValue : value;
}

function TopLevelNavItem({
	href,
	children,
}: {
	href: string;
	children: React.ReactNode;
}) {
	return (
		<li className="md:hidden">
			<CloseButton
				as={Link}
				href={href}
				className="block py-1 text-sm text-zinc-600 transition hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-white"
			>
				{children}
			</CloseButton>
		</li>
	);
}

function NavLink({
	href,
	children,
	tag,
	active = false,
	isAnchorLink = false,
}: {
	href: string;
	children: React.ReactNode;
	tag?: string;
	active?: boolean;
	isAnchorLink?: boolean;
}) {
	return (
		<CloseButton
			as={Link}
			href={href}
			aria-current={active ? "page" : undefined}
			className={clsx(
				"flex justify-between gap-2 py-1 pr-3 text-sm transition",
				isAnchorLink ? "pl-7" : "pl-4",
				active
					? "text-zinc-900 dark:text-white"
					: "text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-white",
			)}
		>
			<span className="truncate">{children}</span>
			{tag && (
				<Tag variant="small" color="zinc">
					{tag}
				</Tag>
			)}
		</CloseButton>
	);
}

function VisibleSectionHighlight({
	group,
	pathname,
}: {
	group: NavGroup;
	pathname: string;
}) {
	const [sections, visibleSections] = useInitialValue(
		[
			useSectionStore((s) => s.sections),
			useSectionStore((s) => s.visibleSections),
		],
		useIsInsideMobileNavigation(),
	);

	const isPresent = useIsPresent();
	const firstVisibleSectionIndex = Math.max(
		0,
		[{ id: "_top" }, ...sections].findIndex(
			(section) => section.id === visibleSections[0],
		),
	);
	const itemHeight = remToPx(2);
	const height = isPresent
		? Math.max(1, visibleSections.length) * itemHeight
		: itemHeight;
	const top =
		group.links.findIndex((link) => link.href === pathname) * itemHeight +
		firstVisibleSectionIndex * itemHeight;

	return (
		<motion.div
			layout
			initial={{ opacity: 0 }}
			animate={{ opacity: 1, transition: { delay: 0.2 } }}
			exit={{ opacity: 0 }}
			className="absolute inset-x-0 top-0 bg-zinc-800/2.5 will-change-transform dark:bg-white/2.5"
			style={{ borderRadius: 8, height, top }}
		/>
	);
}

function ActivePageMarker({
	group,
	pathname,
}: {
	group: NavGroup;
	pathname: string;
}) {
	const itemHeight = remToPx(2);
	const offset = remToPx(0.25);
	const activePageIndex = group.links.findIndex(
		(link) => link.href === pathname,
	);
	const top = offset + activePageIndex * itemHeight;

	return (
		<motion.div
			layout
			className="absolute left-2 h-6 w-px bg-emerald-500"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1, transition: { delay: 0.2 } }}
			exit={{ opacity: 0 }}
			style={{ top }}
		/>
	);
}

function NavigationGroup({
	group,
	className,
}: {
	group: NavGroup;
	className?: string;
}) {
	// If this is the mobile navigation then we always render the initial
	// state, so that the state does not change during the close animation.
	// The state will still update when we re-open (re-render) the navigation.
	const isInsideMobileNavigation = useIsInsideMobileNavigation();
	const [pathname, sections] = useInitialValue(
		[usePathname(), useSectionStore((s) => s.sections)],
		isInsideMobileNavigation,
	);

	const isActiveGroup =
		group.links.findIndex((link) => link.href === pathname) !== -1;

	return (
		<li className={clsx("relative mt-6", className)}>
			<motion.h2
				layout="position"
				className="text-xs font-semibold text-zinc-900 dark:text-white"
			>
				{group.title}
			</motion.h2>
			<div className="relative mt-3 pl-2">
				<AnimatePresence initial={!isInsideMobileNavigation}>
					{isActiveGroup && (
						<VisibleSectionHighlight group={group} pathname={pathname} />
					)}
				</AnimatePresence>
				<motion.div
					layout
					className="absolute inset-y-0 left-2 w-px bg-zinc-900/10 dark:bg-white/5"
				/>
				<AnimatePresence initial={false}>
					{isActiveGroup && (
						<ActivePageMarker group={group} pathname={pathname} />
					)}
				</AnimatePresence>
				<ul className="border-l border-transparent">
					{group.links.map((link) => (
						<motion.li key={link.href} layout="position" className="relative">
							<NavLink href={link.href} active={link.href === pathname}>
								{link.title}
							</NavLink>
							<AnimatePresence mode="popLayout" initial={false}>
								{link.href === pathname && sections.length > 0 && (
									<motion.ul
										initial={{ opacity: 0 }}
										animate={{
											opacity: 1,
											transition: { delay: 0.1 },
										}}
										exit={{
											opacity: 0,
											transition: { duration: 0.15 },
										}}
									>
										{sections.map((section, sectionIndex) => (
											<li key={section.id || `section-${sectionIndex}`}>
												<NavLink
													href={`${link.href}#${section.id}`}
													tag={section.tag}
													isAnchorLink
												>
													{section.title}
												</NavLink>
											</li>
										))}
									</motion.ul>
								)}
							</AnimatePresence>
						</motion.li>
					))}
				</ul>
			</div>
		</li>
	);
}

const zhNavigation: Array<NavGroup> = [
	{
		title: "入门",
		links: [
			{ title: "快速开始", href: withPrefix("/zh/librefang") },
			{ title: "发布路线图", href: withPrefix("/zh/roadmap") },
			{ title: "使用示例", href: withPrefix("/zh/examples") },
			{ title: "术语表", href: withPrefix("/zh/glossary") },
			{ title: "横向对比", href: withPrefix("/zh/comparison") },
		],
	},
	{
		title: "配置",
		links: [
			{ title: "配置文件", href: withPrefix("/zh/configuration") },
			{ title: "LLM 提供商", href: withPrefix("/zh/providers") },
		],
	},
	{
		title: "架构",
		links: [
			{ title: "系统架构", href: withPrefix("/zh/architecture") },
			{ title: "安全", href: withPrefix("/zh/security") },
		],
	},
	{
		title: "Agent",
		links: [
			{ title: "Agent 模板", href: withPrefix("/zh/agents") },
			{ title: "自主 Hands", href: withPrefix("/zh/hands") },
			{ title: "内存系统", href: withPrefix("/zh/memory") },
			{ title: "技能开发", href: withPrefix("/zh/skills") },
			{ title: "工作流", href: withPrefix("/zh/workflows") },
		],
	},
	{
		title: "集成",
		links: [
			{ title: "通道适配器", href: withPrefix("/zh/channels") },
			{ title: "API 参考", href: withPrefix("/zh/api") },
			{ title: "SDK 参考", href: withPrefix("/zh/sdk") },
			{ title: "CLI 参考", href: withPrefix("/zh/cli") },
			{ title: "MCP/A2A", href: withPrefix("/zh/mcp-a2a") },
			{ title: "迁移指南", href: withPrefix("/zh/migration") },
			{ title: "桌面应用", href: withPrefix("/zh/desktop") },
			{ title: "开发指南", href: withPrefix("/zh/development") },
		],
	},
	{
		title: "运维",
		links: [
			{ title: "故障排除", href: withPrefix("/zh/troubleshooting") },
			{ title: "生产部署", href: withPrefix("/zh/production") },
			{ title: "常见问题", href: withPrefix("/zh/faq") },
		],
	},
];

export const enNavigation: Array<NavGroup> = [
	{
		title: "Getting Started",
		links: [
			{ title: "Getting Started", href: withPrefix("/librefang") },
			{ title: "Roadmap", href: withPrefix("/roadmap") },
			{ title: "Examples", href: withPrefix("/examples") },
			{ title: "Glossary", href: withPrefix("/glossary") },
			{ title: "Comparison", href: withPrefix("/comparison") },
		],
	},
	{
		title: "Configuration",
		links: [
			{ title: "Configuration", href: withPrefix("/configuration") },
			{ title: "Providers", href: withPrefix("/providers") },
		],
	},
	{
		title: "Architecture",
		links: [
			{ title: "Architecture", href: withPrefix("/architecture") },
			{ title: "Security", href: withPrefix("/security") },
		],
	},
	{
		title: "Agent",
		links: [
			{ title: "Agent Templates", href: withPrefix("/agents") },
			{ title: "Autonomous Hands", href: withPrefix("/hands") },
			{ title: "Memory System", href: withPrefix("/memory") },
			{ title: "Skills", href: withPrefix("/skills") },
			{ title: "Workflows", href: withPrefix("/workflows") },
		],
	},
	{
		title: "Integrations",
		links: [
			{ title: "Channels", href: withPrefix("/channels") },
			{ title: "API Reference", href: withPrefix("/api") },
			{ title: "SDK Reference", href: withPrefix("/sdk") },
			{ title: "CLI", href: withPrefix("/cli") },
			{ title: "MCP/A2A", href: withPrefix("/mcp-a2a") },
			{ title: "Migration", href: withPrefix("/migration") },
			{ title: "Desktop", href: withPrefix("/desktop") },
			{ title: "Development Guide", href: withPrefix("/development") },
		],
	},
	{
		title: "Operations",
		links: [
			{ title: "Troubleshooting", href: withPrefix("/troubleshooting") },
			{ title: "Production", href: withPrefix("/production") },
			{ title: "FAQ", href: withPrefix("/faq") },
		],
	},
];

export { zhNavigation };

export function Navigation(props: React.ComponentPropsWithoutRef<"nav">) {
	const pathname = usePathname();
	const isZh = pathname?.startsWith("/zh");
	const navigation = isZh ? zhNavigation : enNavigation;

	return (
		<nav {...props}>
			<ul>
				<TopLevelNavItem href={isZh ? withPrefix("/zh") : withPrefix("/")}>
					{isZh ? "文档" : "Docs"}
				</TopLevelNavItem>
				<TopLevelNavItem href="https://github.com/librefang/librefang">
					GitHub
				</TopLevelNavItem>
				{navigation.map((group, groupIndex) => (
					<NavigationGroup
						key={group.title}
						group={group}
						className={groupIndex === 0 ? "md:mt-0" : ""}
					/>
				))}
			</ul>
		</nav>
	);
}
