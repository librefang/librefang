import { motion } from 'framer-motion'
import { ArrowLeft, Github, Loader2 } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { useAppStore } from '../store'
import { cn } from '../lib/utils'

// ---- Types ----

interface GitHubRelease {
  id: number
  tag_name: string
  name: string | null
  body: string | null
  html_url: string
  published_at: string | null
}

// ---- Simple markdown-to-HTML converter ----

function renderMarkdown(md: string): string {
  if (!md) return ''

  let html = md

  // Normalize line endings
  html = html.replace(/\r\n/g, '\n')

  // Fenced code blocks (```lang ... ```)
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_match, _lang: string, code: string) => {
    const escaped = code.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    return '<pre><code>' + escaped + '</code></pre>'
  })

  // Horizontal rules
  html = html.replace(/^---+$/gm, '<hr>')

  // Headers (must come before bold since # lines shouldn't be treated as paragraphs)
  html = html.replace(/^### (.+)$/gm, '<h3>$1</h3>')
  html = html.replace(/^## (.+)$/gm, '<h2>$1</h2>')
  html = html.replace(/^# (.+)$/gm, '<h1>$1</h1>')

  // Bold and italic
  html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
  html = html.replace(/\*(.+?)\*/g, '<em>$1</em>')

  // Inline code (but not inside <pre> blocks)
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>')

  // Links
  html = html.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>'
  )

  // Blockquotes
  html = html.replace(/^> (.+)$/gm, '<blockquote>$1</blockquote>')

  // Unordered lists: collect consecutive lines starting with - or *
  html = html.replace(/((?:^[*-] .+\n?)+)/gm, (block) => {
    const items = block
      .trim()
      .split('\n')
      .map((line) => '<li>' + line.replace(/^[*-] /, '') + '</li>')
      .join('\n')
    return '<ul>' + items + '</ul>\n'
  })

  // Wrap remaining plain-text lines in <p> tags
  const parts = html.split(/\n\n+/)
  html = parts
    .map((part) => {
      const trimmed = part.trim()
      if (!trimmed) return ''
      // Don't wrap block-level elements
      if (/^<(h[1-6]|ul|ol|pre|blockquote|hr|li|div)/.test(trimmed)) return trimmed
      // Don't wrap if it already looks like it's fully wrapped
      if (/^<[a-z]/.test(trimmed) && /<\/[a-z]+>$/.test(trimmed)) return trimmed
      return '<p>' + trimmed.replace(/\n/g, '<br>') + '</p>'
    })
    .join('\n')

  return html
}

// ---- Date formatting ----

function formatDate(dateString: string): string {
  const d = new Date(dateString)
  return d.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric' })
}

// ---- Fetch releases ----

async function fetchReleases(): Promise<GitHubRelease[]> {
  const res = await fetch(
    'https://api.github.com/repos/librefang/librefang/releases?per_page=20',
    { headers: { Accept: 'application/vnd.github.v3+json' } }
  )
  if (!res.ok) {
    throw new Error('GitHub API returned ' + res.status)
  }
  return res.json() as Promise<GitHubRelease[]>
}

// ---- FadeIn wrapper ----

function FadeIn({ children, delay = 0 }: { children: React.ReactNode; delay?: number }) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 24 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, amount: 0.1 }}
      transition={{ duration: 0.6, delay: delay / 1000, ease: 'easeOut' }}
    >
      {children}
    </motion.div>
  )
}

// ---- Main component ----

export default function ChangelogPage() {
  const theme = useAppStore((s) => s.theme)

  const { data: releases, isLoading, error } = useQuery({
    queryKey: ['github-releases'],
    queryFn: fetchReleases,
    staleTime: 5 * 60 * 1000,
  })

  return (
    <div className={cn('min-h-screen bg-surface', theme)}>
      <div className="max-w-[720px] mx-auto px-4 sm:px-6 py-10 sm:py-12">
        {/* Home link */}
        <a
          href="/"
          className="inline-flex items-center gap-1.5 text-sm text-gray-500 hover:text-cyan-500 transition-colors mb-8"
        >
          <ArrowLeft className="w-4 h-4" />
          librefang.ai
        </a>

        {/* Header */}
        <header className="mb-10">
          <h1 className="text-3xl sm:text-4xl font-black tracking-tight mb-2">
            <span className="bg-gradient-to-r from-slate-900 dark:from-white to-cyan-600 dark:to-cyan-400 bg-clip-text text-transparent">
              Changelog
            </span>
          </h1>
          <p className="text-gray-500 text-sm">Release history from GitHub</p>
        </header>

        {/* Loading state */}
        {isLoading && (
          <div className="flex items-center gap-3 py-8 text-gray-500 text-sm">
            <Loader2 className="w-5 h-5 animate-spin text-cyan-500" />
            Loading releases...
          </div>
        )}

        {/* Error state */}
        {error && (
          <div className="bg-red-500/10 border border-red-500/20 rounded-xl px-5 py-4 text-sm text-red-400">
            Failed to load releases: {error instanceof Error ? error.message : 'Unknown error'}
          </div>
        )}

        {/* Releases */}
        {releases && releases.length === 0 && (
          <p className="text-gray-500 text-sm py-8">No releases found.</p>
        )}

        {releases && releases.length > 0 && (
          <div className="space-y-0">
            {releases.map((release, i) => (
              <FadeIn key={release.id} delay={Math.min(i * 80, 400)}>
                <article
                  className={cn(
                    'py-8',
                    i < releases.length - 1 && 'border-b border-black/10 dark:border-white/5'
                  )}
                >
                  {/* Release header */}
                  <div className="flex items-baseline gap-3 flex-wrap mb-3">
                    <a
                      href={release.html_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xl font-bold text-cyan-600 dark:text-cyan-400 font-mono hover:underline"
                    >
                      {release.tag_name}
                    </a>
                    {release.published_at && (
                      <span className="text-sm text-gray-500">
                        {formatDate(release.published_at)}
                      </span>
                    )}
                    <a
                      href={release.html_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="ml-auto text-sm text-gray-500 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors"
                    >
                      View on GitHub &rarr;
                    </a>
                  </div>

                  {/* Release name (if different from tag) */}
                  {release.name && release.name !== release.tag_name && (
                    <div className="text-base font-semibold text-slate-700 dark:text-slate-300 mb-3">
                      {release.name}
                    </div>
                  )}

                  {/* Release body */}
                  {release.body && (
                    <div
                      className={cn(
                        'release-body',
                        'text-slate-600 dark:text-slate-400 leading-relaxed text-[0.9375rem]',
                        // Nested element styles via Tailwind prose-like approach
                        '[&_h1]:text-2xl [&_h1]:font-bold [&_h1]:text-slate-900 [&_h1]:dark:text-white [&_h1]:mt-6 [&_h1]:mb-2',
                        '[&_h2]:text-xl [&_h2]:font-bold [&_h2]:text-slate-900 [&_h2]:dark:text-white [&_h2]:mt-6 [&_h2]:mb-2',
                        '[&_h3]:text-lg [&_h3]:font-semibold [&_h3]:text-slate-800 [&_h3]:dark:text-slate-200 [&_h3]:mt-5 [&_h3]:mb-2',
                        '[&_p]:my-2',
                        '[&_ul]:pl-5 [&_ul]:my-2 [&_ol]:pl-5 [&_ol]:my-2',
                        '[&_li]:mb-1',
                        '[&_strong]:text-slate-800 [&_strong]:dark:text-slate-200 [&_strong]:font-semibold',
                        '[&_code]:font-mono [&_code]:text-[0.85em] [&_code]:bg-surface-200 [&_code]:px-1.5 [&_code]:py-0.5 [&_code]:rounded',
                        '[&_pre]:bg-surface-200 [&_pre]:rounded-lg [&_pre]:px-5 [&_pre]:py-4 [&_pre]:overflow-x-auto [&_pre]:my-3',
                        '[&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-sm [&_pre_code]:leading-relaxed',
                        '[&_a]:text-cyan-600 [&_a]:dark:text-cyan-400 [&_a]:hover:underline',
                        '[&_blockquote]:border-l-[3px] [&_blockquote]:border-black/10 [&_blockquote]:dark:border-white/10 [&_blockquote]:my-3 [&_blockquote]:pl-4 [&_blockquote]:text-gray-500',
                        '[&_hr]:border-0 [&_hr]:border-t [&_hr]:border-black/10 [&_hr]:dark:border-white/5 [&_hr]:my-6',
                      )}
                      dangerouslySetInnerHTML={{ __html: renderMarkdown(release.body) }}
                    />
                  )}
                </article>
              </FadeIn>
            ))}
          </div>
        )}

        {/* Footer */}
        <footer className="text-center py-8 mt-8 text-sm text-gray-500">
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
            <span className="text-gray-700">&bull;</span>
            <a href="/" className="hover:text-cyan-500 transition-colors">
              Website
            </a>
            <span className="text-gray-700">&bull;</span>
            <a
              href="https://discord.gg/DzTYqAZZmc"
              target="_blank"
              rel="noopener noreferrer"
              className="hover:text-cyan-500 transition-colors"
            >
              Discord
            </a>
          </div>
          <p className="text-gray-600">
            &copy; {new Date().getFullYear()} LibreFang &mdash; Agent Operating System
          </p>
        </footer>
      </div>
    </div>
  )
}
