import { cn } from '../lib/utils'
import { useAppStore } from '../store'
import { translations } from '../i18n'
import { ArrowLeft } from 'lucide-react'

export interface Crumb {
  label: string
  href?: string
}

interface BreadcrumbsProps {
  crumbs: Crumb[]
  className?: string
}

// Breadcrumb strip rendered in page content (not inside the fixed header),
// so the header stays byte-for-byte identical across homepage and subpages.
// The first segment is always "Home → ..." linking to the language-aware
// landing page.
export default function Breadcrumbs({ crumbs, className }: BreadcrumbsProps) {
  const lang = useAppStore(s => s.lang)
  const t = translations[lang] || translations['en']!
  const homeHref = lang === 'en' ? '/' : `/${lang}/`
  return (
    <nav aria-label="Breadcrumb" className={cn('flex items-center gap-1.5 text-sm text-gray-500 min-w-0 overflow-x-auto whitespace-nowrap', className)}>
      <a href={homeHref} className="inline-flex items-center gap-1 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors shrink-0">
        <ArrowLeft className="w-3.5 h-3.5" />
        {t.registry?.backHome || 'Home'}
      </a>
      {crumbs.map((c, i) => {
        const isLast = i === crumbs.length - 1
        return (
          <span key={i} className="flex items-center gap-1.5 min-w-0">
            <span className="text-gray-300 dark:text-gray-700 shrink-0">/</span>
            {isLast || !c.href ? (
              <span className={cn('truncate', isLast ? 'text-slate-900 dark:text-white font-semibold' : '')}>{c.label}</span>
            ) : (
              <a href={c.href} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors truncate">{c.label}</a>
            )}
          </span>
        )
      })}
    </nav>
  )
}
