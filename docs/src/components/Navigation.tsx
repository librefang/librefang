"use client";

import { CloseButton } from "@headlessui/react";
import clsx from "clsx";
import { ChevronRight } from "lucide-react";
import { AnimatePresence, motion, useIsPresent } from "motion/react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { useIsInsideMobileNavigation } from "@/components/MobileNavigation";
import { type Section, useSectionStore } from "@/components/SectionProvider";
import { Tag } from "@/components/Tag";
import { remToPx } from "@/lib/remToPx";
import { withPrefix } from "@/lib/utils";

const AllSectionsContext = createContext<
	Record<string, Array<Section>> | undefined
>(undefined);

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

function lookupSections(
	allSections: Record<string, Array<Section>> | undefined,
	href: string,
): Array<Section> | undefined {
	if (!allSections) return undefined;
	const normalized = href.replace(/^\/?/, "/");
	return allSections[normalized] ?? allSections[`${normalized}/`];
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
	indicator,
}: {
	href: string;
	children: React.ReactNode;
	tag?: string;
	active?: boolean;
	isAnchorLink?: boolean;
	indicator?: React.ReactNode;
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
			{indicator}
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
	allSections,
	isOpen,
	onToggle,
}: {
	group: NavGroup;
	className?: string;
	allSections?: Record<string, Array<Section>>;
	isOpen: boolean;
	onToggle: (navigateToFirst?: boolean) => void;
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

	const [collapsedSections, setCollapsedSections] = useState<Set<string>>(
		new Set(),
	);

	const toggleSections = (href: string) => {
		setCollapsedSections((prev) => {
			const next = new Set(prev);
			if (next.has(href)) {
				next.delete(href);
			} else {
				next.add(href);
			}
			return next;
		});
	};

	return (
		<li className={clsx("relative mt-6", className)}>
			<motion.h2
				layout="position"
				className="flex cursor-pointer select-none items-center gap-1.5 text-sm font-semibold text-zinc-900 dark:text-white"
				onClick={() => onToggle(true)}
			>
				<ChevronRight
					className={clsx(
						"h-3.5 w-3.5 shrink-0 text-zinc-500 transition-transform duration-200 dark:text-zinc-400",
						isOpen && "rotate-90",
					)}
				/>
				{group.title}
			</motion.h2>
			<AnimatePresence initial={false}>
				{isOpen && (
					<motion.div
						className="relative mt-3 pl-2 overflow-hidden"
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1, transition: { duration: 0.2 } }}
						exit={{ height: 0, opacity: 0, transition: { duration: 0.15 } }}
					>
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
							{group.links.map((link) => {
								const isActive = link.href === pathname;
								const hasSections = isActive && sections.length > 0;
								const isExpanded = hasSections && !collapsedSections.has(link.href);

								return (
									<motion.li key={link.href} layout="position" className="relative">
										<NavLink
											href={link.href}
											active={isActive}
											indicator={
												lookupSections(allSections, link.href)?.length ? (
													<button
														type="button"
														className="flex items-center justify-center p-0.5 -m-0.5"
														onClick={(e) => {
															if (isActive && hasSections) {
																e.preventDefault();
																e.stopPropagation();
																toggleSections(link.href);
															}
														}}
													>
														<ChevronRight
															className={clsx(
																"h-3.5 w-3.5 shrink-0 text-zinc-400 transition-transform duration-200 dark:text-zinc-500",
																isExpanded && "rotate-90",
															)}
														/>
													</button>
												) : null
											}
										>
											{link.title}
										</NavLink>
										<AnimatePresence mode="popLayout" initial={false}>
											{isExpanded && (
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
								);
							})}
						</ul>
					</motion.div>
				)}
			</AnimatePresence>
		</li>
	);
}

const zhNavigation: Array<NavGroup> = [
	{
		title: "入门",
		links: [
			{ title: "快速开始", href: withPrefix("/zh/getting-started") },
			{ title: "发布路线图", href: withPrefix("/zh/getting-started/roadmap") },
			{ title: "使用示例", href: withPrefix("/zh/getting-started/examples") },
			{ title: "术语表", href: withPrefix("/zh/getting-started/glossary") },
			{ title: "横向对比", href: withPrefix("/zh/getting-started/comparison") },
		],
	},
	{
		title: "配置",
		links: [
			{ title: "配置文件", href: withPrefix("/zh/configuration") },
			{ title: "核心配置", href: withPrefix("/zh/configuration/core") },
			{ title: "通道配置", href: withPrefix("/zh/configuration/channels") },
			{ title: "功能配置", href: withPrefix("/zh/configuration/features") },
			{ title: "Cron 调度器", href: withPrefix("/zh/configuration/features/cron") },
			{ title: "会话自动重置", href: withPrefix("/zh/configuration/features/session-reset") },
			{ title: "安全配置", href: withPrefix("/zh/configuration/security") },
			{ title: "LLM 提供商", href: withPrefix("/zh/configuration/providers") },
			{ title: "托管 API", href: withPrefix("/zh/configuration/providers/hosted") },
			{ title: "本地与自托管", href: withPrefix("/zh/configuration/providers/local") },
			{ title: "平台与托管端点", href: withPrefix("/zh/configuration/providers/platforms") },
			{ title: "开发工具", href: withPrefix("/zh/configuration/providers/tools") },
			{ title: "提供商管理", href: withPrefix("/zh/configuration/providers/management") },
		],
	},
	{
		title: "架构",
		links: [
			{ title: "系统架构", href: withPrefix("/zh/architecture") },
			{ title: "安全", href: withPrefix("/zh/architecture/security") },
			{ title: "OFP 链路加密", href: withPrefix("/zh/architecture/ofp-wire") },
		],
	},
	{
		title: "安全",
		links: [
			{ title: "沙盒与执行", href: withPrefix("/zh/security/sandboxing") },
			{ title: "完整性与凭据", href: withPrefix("/zh/security/integrity") },
			{ title: "网络与 API 安全", href: withPrefix("/zh/security/network-api") },
			{ title: "运维与防护", href: withPrefix("/zh/security/operations") },
			{ title: "审批与 TOTP", href: withPrefix("/zh/security/approvals") },
		],
	},
	{
		title: "智能体",
		links: [
			{ title: "Agent 模板", href: withPrefix("/zh/agent/templates") },
			{ title: "自主 Hands", href: withPrefix("/zh/agent/hands") },
			{ title: "内存系统", href: withPrefix("/zh/agent/memory") },
			{ title: "技能开发", href: withPrefix("/zh/agent/skills") },
			{ title: "插件开发", href: withPrefix("/zh/agent/plugins") },
			{ title: "事件 Hooks", href: withPrefix("/zh/agent/hooks") },
			{ title: "内置工具", href: withPrefix("/zh/agent/tools") },
			{ title: "Prompt 智能", href: withPrefix("/zh/agent/prompt-intelligence") },
			{ title: "工作流", href: withPrefix("/zh/agent/workflows") },
		],
	},
	{
		title: "集成",
		links: [
			{ title: "通道适配器", href: withPrefix("/zh/integrations/channels") },
			{ title: "核心消息", href: withPrefix("/zh/integrations/channels/core") },
			{ title: "企业协作", href: withPrefix("/zh/integrations/channels/enterprise") },
			{ title: "社交与社区", href: withPrefix("/zh/integrations/channels/social") },
			{ title: "集成与协议", href: withPrefix("/zh/integrations/channels/integrations") },
			{ title: "自定义适配器", href: withPrefix("/zh/integrations/channels/custom") },
			{ title: "API 参考", href: withPrefix("/zh/integrations/api") },
			{ title: "代理与工作流 API", href: withPrefix("/zh/integrations/api/agents") },
			{ title: "系统与配置 API", href: withPrefix("/zh/integrations/api/system") },
			{ title: "智能与技能 API", href: withPrefix("/zh/integrations/api/intelligence") },
			{ title: "供应商与模型 API", href: withPrefix("/zh/integrations/api/providers") },
			{ title: "通信与网络 API", href: withPrefix("/zh/integrations/api/communication") },
			{ title: "实时 API", href: withPrefix("/zh/integrations/api/realtime") },
			{ title: "SDK 参考", href: withPrefix("/zh/integrations/sdk") },
			{ title: "CLI 参考", href: withPrefix("/zh/integrations/cli") },
			{ title: "CLI 命令", href: withPrefix("/zh/integrations/cli/commands") },
			{ title: "CLI 示例", href: withPrefix("/zh/integrations/cli/examples") },
			{ title: "Android / Termux", href: withPrefix("/zh/integrations/android-termux") },
			{ title: "MCP/A2A", href: withPrefix("/zh/integrations/mcp-a2a") },
			{ title: "迁移指南", href: withPrefix("/zh/integrations/migration") },
			{ title: "桌面应用", href: withPrefix("/zh/integrations/desktop") },
			{ title: "开发指南", href: withPrefix("/zh/integrations/development") },
		],
	},
	{
		title: "运维",
		links: [
			{ title: "故障排除", href: withPrefix("/zh/operations/troubleshooting") },
			{ title: "检查点管理", href: withPrefix("/zh/operations/checkpoints") },
			{ title: "生产部署", href: withPrefix("/zh/operations/production") },
			{ title: "常见问题", href: withPrefix("/zh/operations/faq") },
		],
	},
];

export const enNavigation: Array<NavGroup> = [
	{
		title: "Getting Started",
		links: [
			{ title: "Getting Started", href: withPrefix("/getting-started") },
			{ title: "Roadmap", href: withPrefix("/getting-started/roadmap") },
			{ title: "Examples", href: withPrefix("/getting-started/examples") },
			{ title: "Glossary", href: withPrefix("/getting-started/glossary") },
			{ title: "Comparison", href: withPrefix("/getting-started/comparison") },
		],
	},
	{
		title: "Configuration",
		links: [
			{ title: "Configuration", href: withPrefix("/configuration") },
			{ title: "Core Config", href: withPrefix("/configuration/core") },
			{ title: "Channel Config", href: withPrefix("/configuration/channels") },
			{ title: "Feature Config", href: withPrefix("/configuration/features") },
			{ title: "Cron Scheduler", href: withPrefix("/configuration/features/cron") },
			{ title: "Session Auto-Reset", href: withPrefix("/configuration/features/session-reset") },
			{ title: "Security Config", href: withPrefix("/configuration/security") },
			{ title: "Providers", href: withPrefix("/configuration/providers") },
			{ title: "Hosted APIs", href: withPrefix("/configuration/providers/hosted") },
			{ title: "Local & Self-Hosted", href: withPrefix("/configuration/providers/local") },
			{ title: "Platforms & Managed", href: withPrefix("/configuration/providers/platforms") },
			{ title: "Developer Tools", href: withPrefix("/configuration/providers/tools") },
			{ title: "Provider Management", href: withPrefix("/configuration/providers/management") },
		],
	},
	{
		title: "Architecture",
		links: [
			{ title: "Architecture", href: withPrefix("/architecture") },
			{ title: "Security", href: withPrefix("/architecture/security") },
			{ title: "OFP Wire Encryption", href: withPrefix("/architecture/ofp-wire") },
		],
	},
	{
		title: "Security",
		links: [
			{ title: "Sandboxing", href: withPrefix("/security/sandboxing") },
			{ title: "Integrity & Secrets", href: withPrefix("/security/integrity") },
			{ title: "Network & API", href: withPrefix("/security/network-api") },
			{ title: "Operations", href: withPrefix("/security/operations") },
			{ title: "Approvals & TOTP", href: withPrefix("/security/approvals") },
		],
	},
	{
		title: "Agent",
		links: [
			{ title: "Agent Templates", href: withPrefix("/agent/templates") },
			{ title: "Autonomous Hands", href: withPrefix("/agent/hands") },
			{ title: "Memory System", href: withPrefix("/agent/memory") },
			{ title: "Skills", href: withPrefix("/agent/skills") },
			{ title: "Plugins", href: withPrefix("/agent/plugins") },
			{ title: "Event Hooks", href: withPrefix("/agent/hooks") },
			{ title: "Built-in Tools", href: withPrefix("/agent/tools") },
			{ title: "Prompt Intelligence", href: withPrefix("/agent/prompt-intelligence") },
			{ title: "Workflows", href: withPrefix("/agent/workflows") },
		],
	},
	{
		title: "Integrations",
		links: [
			{ title: "Channels", href: withPrefix("/integrations/channels") },
			{ title: "Core Messaging", href: withPrefix("/integrations/channels/core") },
			{ title: "Enterprise", href: withPrefix("/integrations/channels/enterprise") },
			{ title: "Social & Community", href: withPrefix("/integrations/channels/social") },
			{ title: "Integrations", href: withPrefix("/integrations/channels/integrations") },
			{ title: "Custom Adapters", href: withPrefix("/integrations/channels/custom") },
			{ title: "API Reference", href: withPrefix("/integrations/api") },
			{ title: "Agent & Workflow API", href: withPrefix("/integrations/api/agents") },
			{ title: "System & Config API", href: withPrefix("/integrations/api/system") },
			{ title: "Intelligence API", href: withPrefix("/integrations/api/intelligence") },
			{ title: "Provider & Model API", href: withPrefix("/integrations/api/providers") },
			{ title: "Communication API", href: withPrefix("/integrations/api/communication") },
			{ title: "Real-time API", href: withPrefix("/integrations/api/realtime") },
			{ title: "SDK Reference", href: withPrefix("/integrations/sdk") },
			{ title: "CLI", href: withPrefix("/integrations/cli") },
			{ title: "CLI Commands", href: withPrefix("/integrations/cli/commands") },
			{ title: "CLI Examples", href: withPrefix("/integrations/cli/examples") },
			{ title: "Android / Termux", href: withPrefix("/integrations/android-termux") },
			{ title: "MCP/A2A", href: withPrefix("/integrations/mcp-a2a") },
			{ title: "Migration", href: withPrefix("/integrations/migration") },
			{ title: "Desktop", href: withPrefix("/integrations/desktop") },
			{ title: "Development Guide", href: withPrefix("/integrations/development") },
		],
	},
	{
		title: "Operations",
		links: [
			{ title: "Troubleshooting", href: withPrefix("/operations/troubleshooting") },
			{ title: "Checkpoints", href: withPrefix("/operations/checkpoints") },
			{ title: "Production", href: withPrefix("/operations/production") },
			{ title: "FAQ", href: withPrefix("/operations/faq") },
		],
	},
];

export { zhNavigation };

export function AllSectionsProvider({
	allSections,
	children,
}: {
	allSections: Record<string, Array<Section>>;
	children: React.ReactNode;
}) {
	return (
		<AllSectionsContext.Provider value={allSections}>
			{children}
		</AllSectionsContext.Provider>
	);
}

export function Navigation({
	allSections: allSectionsProp,
	...props
}: React.ComponentPropsWithoutRef<"nav"> & {
	allSections?: Record<string, Array<Section>>;
}) {
	const allSectionsCtx = useContext(AllSectionsContext);
	const allSections = allSectionsProp ?? allSectionsCtx;
	const pathname = usePathname();
	const isZh = pathname?.startsWith("/zh");
	const navigation = isZh ? zhNavigation : enNavigation;

	// Accordion: find group containing the active page
	const activeGroupIndex = navigation.findIndex((group) =>
		group.links.some((link) => link.href === pathname),
	);
	const [openGroupIndex, setOpenGroupIndex] = useState(
		activeGroupIndex !== -1 ? activeGroupIndex : 0,
	);

	// Sync when pathname changes
	useEffect(() => {
		const idx = navigation.findIndex((group) =>
			group.links.some((link) => link.href === pathname),
		);
		if (idx !== -1) {
			setOpenGroupIndex(idx);
		}
	}, [pathname, navigation]);

	const router = useRouter();
	const handleToggle = useCallback(
		(index: number, navigateToFirst?: boolean) => {
			setOpenGroupIndex((prev) => (prev === index ? -1 : index));
			if (navigateToFirst && navigation[index]?.links[0]) {
				router.push(navigation[index].links[0].href);
			}
		},
		[navigation, router],
	);

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
						allSections={allSections}
						isOpen={openGroupIndex === groupIndex}
						onToggle={(navigateToFirst) => handleToggle(groupIndex, navigateToFirst)}
					/>
				))}
			</ul>
		</nav>
	);
}
