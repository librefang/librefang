import { useState, useMemo } from 'react'
import { motion } from 'framer-motion'
import { ArrowLeft, ExternalLink, Download, Tag, Loader2, Github, Filter } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

// ---- Types ----

interface ReleaseAsset {
  name: string
  download_count: number
  browser_download_url: string
}

interface GitHubRelease {
  id: number
  tag_name: string
  name: string | null
  body: string | null
  html_url: string
  published_at: string | null
  prerelease: boolean
  draft: boolean
  assets: ReleaseAsset[]
}

type ChangeType = 'feature' | 'fix' | 'breaking' | 'performance' | 'other'

interface ParsedChange {
  type: ChangeType
  text: string
}

interface ParsedRelease {
  release: GitHubRelease
  changes: Map<ChangeType, ParsedChange[]>
  totalDownloads: number
  releaseType: 'stable' | 'rc' | 'beta'
}

type FilterType = 'all' | 'stable' | 'prerelease'

// ---- Constants ----

const TYPE_CONFIG: Record<ChangeType, { label: string; dotClass: string; badgeClass: string }> = {
  feature: {
    label: 'Added',
    dotClass: 'bg-cyan-500',
    badgeClass: 'text-cyan-700 dark:text-cyan-300 bg-cyan-500/10 border-cyan-500/20',
  },
  fix: {
    label: 'Fixed',
    dotClass: 'bg-amber-500',
    badgeClass: 'text-amber-700 dark:text-amber-300 bg-amber-500/10 border-amber-500/20',
  },
  breaking: {
    label: 'Breaking',
    dotClass: 'bg-red-500',
    badgeClass: 'text-red-700 dark:text-red-300 bg-red-500/10 border-red-500/20',
  },
  performance: {
    label: 'Performance',
    dotClass: 'bg-purple-500',
    badgeClass: 'text-purple-700 dark:text-purple-300 bg-purple-500/10 border-purple-500/20',
  },
  other: {
    label: 'Other',
    dotClass: 'bg-gray-400 dark:bg-gray-500',
    badgeClass: 'text-gray-600 dark:text-gray-400 bg-gray-500/10 border-gray-500/20',
  },
}

const RELEASE_TYPE_BADGE: Record<string, { label: string; className: string }> = {
  stable: {
    label: 'Stable',
    className: 'text-emerald-700 dark:text-emerald-300 bg-emerald-500/10 border-emerald-500/20',
  },
  rc: {
    label: 'RC',
    className: 'text-amber-700 dark:text-amber-300 bg-amber-500/10 border-amber-500/20',
  },
  beta: {
    label: 'Beta',
    className: 'text-violet-700 dark:text-violet-300 bg-violet-500/10 border-violet-500/20',
  },
}

// Display order for change categories
const CATEGORY_ORDER: ChangeType[] = ['breaking', 'feature', 'fix', 'performance', 'other']

// ---- Parsing helpers ----

function detectReleaseType(tag: string): 'stable' | 'rc' | 'beta' {
  const lower = tag.toLowerCase()
  if (lower.includes('-beta')) return 'beta'
  if (lower.includes('-rc')) return 'rc'
  return 'stable'
}

function parseChanges(body: string | null): Map<ChangeType, ParsedChange[]> {
  const grouped = new Map<ChangeType, ParsedChange[]>()
  if (!body) return grouped

  const lines = body.split('\n')
  let currentSection: ChangeType | null = null

  // Track which sections we've seen — the body uses ### headings
  // like "### Added", "### Fixed", "### Performance", etc.
  const sectionMap: Record<string, ChangeType> = {
    added: 'feature',
    'new features': 'feature',
    features: 'feature',
    feat: 'feature',
    fixed: 'fix',
    'bug fixes': 'fix',
    fixes: 'fix',
    breaking: 'breaking',
    'breaking changes': 'breaking',
    performance: 'performance',
    perf: 'performance',
    changed: 'other',
    other: 'other',
    chore: 'other',
    docs: 'other',
    documentation: 'other',
    refactor: 'other',
    deprecated: 'other',
    removed: 'other',
    security: 'fix',
  }

  for (const line of lines) {
    const trimmed = line.trim()

    // Detect section headers: ### Added, ### Fixed, etc.
    const headerMatch = trimmed.match(/^#{1,3}\s+(.+)$/)
    if (headerMatch) {
      const heading = headerMatch[1]!.toLowerCase().trim()
      if (sectionMap[heading] !== undefined) {
        currentSection = sectionMap[heading]!
      }
      continue
    }

    // Detect list items: - item or * item
    const listMatch = trimmed.match(/^[*-]\s+(.+)$/)
    if (listMatch && currentSection) {
      const text = listMatch[1]!.trim()
      if (text.length < 3) continue

      // Skip section dividers, installation instructions, links
      if (text.startsWith('```') || text.startsWith('[') || text.startsWith('http')) continue

      const change: ParsedChange = { type: currentSection, text }
      const existing = grouped.get(currentSection)
      if (existing) {
        existing.push(change)
      } else {
        grouped.set(currentSection, [change])
      }
      continue
    }

    // If we hit non-list, non-header content after a section, and it looks like
    // a new major heading (## Something), reset
    if (trimmed.match(/^##\s+/) && !trimmed.match(/^###/)) {
      // Check for known non-changelog sections
      const heading = trimmed.replace(/^##\s+/, '').toLowerCase()
      if (['installation', 'links', 'contributors'].some((s) => heading.includes(s))) {
        currentSection = null
      }
    }
  }

  // Fallback: if no sections found, try to parse lines with conventional commit prefixes
  if (grouped.size === 0) {
    for (const line of lines) {
      const trimmed = line.replace(/^[*-]\s*/, '').trim()
      if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('---') || trimmed.length < 5) continue

      let type: ChangeType = 'other'
      if (/^feat[:(]/i.test(trimmed)) type = 'feature'
      else if (/^fix[:(]/i.test(trimmed)) type = 'fix'
      else if (/^break/i.test(trimmed)) type = 'breaking'
      else if (/^perf[:(]/i.test(trimmed)) type = 'performance'
      else if (/^(chore|ci|docs|refactor|test|build)[:(]/i.test(trimmed)) type = 'other'
      else continue // skip lines that don't match any pattern

      const change: ParsedChange = { type, text: trimmed }
      const existing = grouped.get(type)
      if (existing) {
        existing.push(change)
      } else {
        grouped.set(type, [change])
      }
    }
  }

  return grouped
}

function getTotalDownloads(release: GitHubRelease): number {
  return release.assets.reduce((sum, a) => sum + a.download_count, 0)
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

/**
 * Auto-link #123 references and @user mentions within a change line.
 * Returns an HTML string.
 */
function linkify(text: string): string {
  let result = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')

  // #issue
  result = result.replace(
    /(?<![&\w])#(\d+)/g,
    '<a href="https://github.com/librefang/librefang/issues/$1" target="_blank" rel="noopener noreferrer" class="text-cyan-600 dark:text-cyan-400 hover:underline">#$1</a>',
  )
  // @user
  result = result.replace(
    /(?<![&\w])@([a-zA-Z0-9-]+)/g,
    '<a href="https://github.com/$1" target="_blank" rel="noopener noreferrer" class="text-cyan-600 dark:text-cyan-400 hover:underline">@$1</a>',
  )
  return result
}

// ---- Data fetching ----

async function fetchReleases(): Promise<GitHubRelease[]> {
  const res = await fetch('https://stats.librefang.ai/api/releases')
  if (!res.ok) {
    throw new Error('Failed to load releases: ' + res.status)
  }
  return res.json() as Promise<GitHubRelease[]>
}

// ---- Sub-components ----

function TimelineDot({ isFirst }: { isFirst: boolean }) {
  return (
    <div className="relative flex flex-col items-center">
      {/* The dot */}
      <div
        className={cn(
          'w-3 h-3 rounded-full border-2 border-surface bg-cyan-500 z-10',
          isFirst && 'w-4 h-4 ring-4 ring-cyan-500/20',
        )}
      />
    </div>
  )
}

function ReleaseBadge({ type }: { type: 'stable' | 'rc' | 'beta' }) {
  const config = RELEASE_TYPE_BADGE[type]!
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium border',
        config.className,
      )}
    >
      <Tag className="w-3 h-3" />
      {config.label}
    </span>
  )
}

function ChangeCategory({
  type,
  changes,
}: {
  type: ChangeType
  changes: ParsedChange[]
}) {
  const config = TYPE_CONFIG[type]
  return (
    <div>
      <div className="flex items-center gap-2 mb-2">
        <span className={cn('w-2 h-2 rounded-full', config.dotClass)} />
        <h4
          className={cn(
            'text-sm font-semibold inline-flex items-center gap-1.5 px-2 py-0.5 rounded border',
            config.badgeClass,
          )}
        >
          {config.label}
        </h4>
        <span className="text-xs text-gray-500">{changes.length}</span>
      </div>
      <ul className="space-y-1 ml-4 pl-4 border-l border-black/5 dark:border-white/5">
        {changes.map((change, i) => (
          <li key={i} className="text-sm text-slate-700 dark:text-slate-300 leading-relaxed">
            <span dangerouslySetInnerHTML={{ __html: linkify(change.text) }} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function ReleaseCard({
  parsed,
  index,
  isFirst,
}: {
  parsed: ParsedRelease
  index: number
  isFirst: boolean
}) {
  const { release, changes, totalDownloads, releaseType } = parsed
  const orderedCategories = CATEGORY_ORDER.filter((t) => changes.has(t))

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, amount: 0.15 }}
      transition={{ duration: 0.5, delay: Math.min(index * 0.06, 0.3), ease: 'easeOut' }}
      className="relative grid grid-cols-[40px_1fr] md:grid-cols-[80px_1fr] gap-0 md:gap-4"
    >
      {/* Timeline column */}
      <div className="flex flex-col items-center">
        <TimelineDot isFirst={isFirst} />
        {/* Vertical line extends down */}
        <div className="w-px flex-1 bg-black/10 dark:bg-white/10" />
      </div>

      {/* Card column */}
      <div className="pb-8 md:pb-10">
        <div
          className={cn(
            'rounded-xl border border-black/10 dark:border-white/5',
            'bg-surface-100 hover:bg-surface-200/50 transition-colors duration-200',
            isFirst && 'glow-cyan',
          )}
        >
          {/* Card header */}
          <div className="px-4 py-3 sm:px-5 sm:py-4 border-b border-black/5 dark:border-white/5">
            <div className="flex flex-wrap items-center gap-2 sm:gap-3">
              <a
                href={release.html_url}
                target="_blank"
                rel="noopener noreferrer"
                className="text-lg sm:text-xl font-bold font-mono text-cyan-600 dark:text-cyan-400 hover:underline"
              >
                {release.tag_name}
              </a>
              <ReleaseBadge type={releaseType} />
              {release.published_at && (
                <span className="text-xs sm:text-sm text-gray-500">
                  {formatDate(release.published_at)}
                </span>
              )}
              {totalDownloads > 0 && (
                <span className="inline-flex items-center gap-1 text-xs text-gray-500 ml-auto">
                  <Download className="w-3.5 h-3.5" />
                  {totalDownloads.toLocaleString()}
                </span>
              )}
            </div>
            {/* Release name if different from tag */}
            {release.name && release.name !== release.tag_name && (
              <p className="text-sm text-slate-600 dark:text-slate-400 mt-1">{release.name}</p>
            )}
          </div>

          {/* Card body: grouped changes */}
          {orderedCategories.length > 0 && (
            <div className="px-4 py-3 sm:px-5 sm:py-4 space-y-4">
              {orderedCategories.map((type) => (
                <ChangeCategory key={type} type={type} changes={changes.get(type)!} />
              ))}
            </div>
          )}

          {/* If no parsed changes but body exists, show a note */}
          {orderedCategories.length === 0 && release.body && (
            <div className="px-4 py-3 sm:px-5 sm:py-4">
              <p className="text-sm text-gray-500 italic">
                See full release notes on GitHub.
              </p>
            </div>
          )}

          {/* Card footer */}
          <div className="px-4 py-2.5 sm:px-5 sm:py-3 border-t border-black/5 dark:border-white/5 flex items-center justify-end">
            <a
              href={release.html_url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-xs sm:text-sm text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
            >
              View on GitHub
              <ExternalLink className="w-3.5 h-3.5" />
            </a>
          </div>
        </div>
      </div>
    </motion.div>
  )
}

// ---- Filter tabs ----

function FilterTabs({
  active,
  onChange,
  counts,
}: {
  active: FilterType
  onChange: (f: FilterType) => void
  counts: { all: number; stable: number; prerelease: number }
}) {
  const tabs: { key: FilterType; label: string; count: number }[] = [
    { key: 'all', label: 'All', count: counts.all },
    { key: 'stable', label: 'Stable', count: counts.stable },
    { key: 'prerelease', label: 'Pre-release', count: counts.prerelease },
  ]

  return (
    <div className="flex items-center gap-1 p-1 rounded-lg bg-surface-100 border border-black/5 dark:border-white/5">
      {tabs.map((tab) => (
        <button
          key={tab.key}
          onClick={() => onChange(tab.key)}
          className={cn(
            'px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-500/50',
            active === tab.key
              ? 'bg-surface-200 text-slate-900 dark:text-white shadow-sm'
              : 'text-gray-500 hover:text-slate-700 dark:hover:text-slate-300',
          )}
        >
          {tab.label}
          <span
            className={cn(
              'ml-1.5 text-xs tabular-nums',
              active === tab.key ? 'text-cyan-600 dark:text-cyan-400' : 'text-gray-400',
            )}
          >
            {tab.count}
          </span>
        </button>
      ))}
    </div>
  )
}

// ---- Main component ----

export default function ChangelogPage() {
  const theme = useAppStore((s) => s.theme)
  const [filter, setFilter] = useState<FilterType>('all')

  const {
    data: releases,
    isLoading,
    error,
  } = useQuery({
    queryKey: ['github-releases'],
    queryFn: fetchReleases,
    staleTime: 5 * 60 * 1000,
  })

  // Parse all releases into structured data
  const parsedReleases = useMemo(() => {
    if (!releases) return []
    return releases.map(
      (release): ParsedRelease => ({
        release,
        changes: parseChanges(release.body),
        totalDownloads: getTotalDownloads(release),
        releaseType: detectReleaseType(release.tag_name),
      }),
    )
  }, [releases])

  // Count by type for filter tabs
  const counts = useMemo(() => {
    const all = parsedReleases.length
    const stable = parsedReleases.filter((p) => p.releaseType === 'stable').length
    const prerelease = all - stable
    return { all, stable, prerelease }
  }, [parsedReleases])

  // Apply filter
  const filtered = useMemo(() => {
    if (filter === 'all') return parsedReleases
    if (filter === 'stable') return parsedReleases.filter((p) => p.releaseType === 'stable')
    return parsedReleases.filter((p) => p.releaseType !== 'stable')
  }, [parsedReleases, filter])

  // Total downloads across all releases
  const totalDownloads = useMemo(
    () => parsedReleases.reduce((sum, p) => sum + p.totalDownloads, 0),
    [parsedReleases],
  )

  return (
    <div className={cn('min-h-screen bg-surface', theme)}>
      <div className="max-w-[860px] mx-auto px-4 sm:px-6 py-8 sm:py-12">
        {/* Navigation */}
        <a
          href="/"
          className="inline-flex items-center gap-1.5 text-sm text-gray-500 hover:text-cyan-500 transition-colors mb-8"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to home
        </a>

        {/* Header */}
        <header className="mb-8">
          <motion.div
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5 }}
          >
            <h1 className="text-3xl sm:text-4xl font-black tracking-tight mb-2">
              <span className="bg-gradient-to-r from-slate-900 dark:from-white to-cyan-600 dark:to-cyan-400 bg-clip-text text-transparent">
                Changelog
              </span>
            </h1>
            <p className="text-gray-500 text-sm sm:text-base">
              Track every update to LibreFang
            </p>
          </motion.div>

          {/* Stats row */}
          {releases && releases.length > 0 && (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.5, delay: 0.1 }}
              className="flex flex-wrap items-center gap-3 sm:gap-4 mt-5"
            >
              <div className="flex items-center gap-4 text-xs sm:text-sm text-gray-500">
                <span className="inline-flex items-center gap-1.5">
                  <Tag className="w-3.5 h-3.5 text-cyan-500" />
                  {parsedReleases.length} releases
                </span>
                {totalDownloads > 0 && (
                  <span className="inline-flex items-center gap-1.5">
                    <Download className="w-3.5 h-3.5 text-cyan-500" />
                    {totalDownloads.toLocaleString()} downloads
                  </span>
                )}
              </div>

              <div className="flex items-center gap-2 ml-auto">
                <Filter className="w-3.5 h-3.5 text-gray-400 hidden sm:block" />
                <FilterTabs active={filter} onChange={setFilter} counts={counts} />
              </div>
            </motion.div>
          )}
        </header>

        {/* Loading state */}
        {isLoading && (
          <div className="flex items-center gap-3 py-16 justify-center text-gray-500 text-sm">
            <Loader2 className="w-5 h-5 animate-spin text-cyan-500" />
            Loading releases...
          </div>
        )}

        {/* Error state */}
        {error && (
          <div className="bg-red-500/10 border border-red-500/20 rounded-xl px-5 py-4 text-sm text-red-600 dark:text-red-400">
            Failed to load releases:{' '}
            {error instanceof Error ? error.message : 'Unknown error'}
          </div>
        )}

        {/* Empty state */}
        {releases && filtered.length === 0 && (
          <div className="text-center py-16">
            <Tag className="w-10 h-10 text-gray-300 dark:text-gray-600 mx-auto mb-3" />
            <p className="text-gray-500 text-sm">
              {releases.length === 0
                ? 'No releases found.'
                : 'No releases match the current filter.'}
            </p>
          </div>
        )}

        {/* Timeline */}
        {filtered.length > 0 && (
          <div className="relative">
            {filtered.map((parsed, i) => (
              <ReleaseCard
                key={parsed.release.id}
                parsed={parsed}
                index={i}
                isFirst={i === 0}
              />
            ))}

            {/* Timeline end cap */}
            <div className="grid grid-cols-[40px_1fr] md:grid-cols-[80px_1fr] gap-0 md:gap-4">
              <div className="flex justify-center">
                <div className="w-2 h-2 rounded-full bg-gray-300 dark:bg-gray-600" />
              </div>
              <div />
            </div>
          </div>
        )}

        {/* Footer */}
        <footer className="text-center py-8 mt-8 text-sm text-gray-500 border-t border-black/5 dark:border-white/5">
          <div className="flex items-center justify-center gap-4 mb-3">
            <a
              href="https://github.com/librefang/librefang"
              target="_blank"
              rel="noopener noreferrer"
              className="hover:text-cyan-500 transition-colors flex items-center gap-1.5"
            >
              <Github className="w-4 h-4" />
              GitHub
            </a>
            <span className="text-gray-300 dark:text-gray-700">&bull;</span>
            <a href="/" className="hover:text-cyan-500 transition-colors">
              Website
            </a>
            <span className="text-gray-300 dark:text-gray-700">&bull;</span>
            <a
              href="https://discord.gg/DzTYqAZZmc"
              target="_blank"
              rel="noopener noreferrer"
              className="hover:text-cyan-500 transition-colors"
            >
              Discord
            </a>
          </div>
          <p className="text-gray-400 dark:text-gray-600">
            &copy; {new Date().getFullYear()} LibreFang &mdash; Agent Operating System
          </p>
        </footer>
      </div>
    </div>
  )
}
