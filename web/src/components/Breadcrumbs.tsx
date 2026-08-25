import { cn } from '../lib/utils'
import { useAppStore } from '../store'
import { getTranslation } from '../i18n'
import type { Translation } from '../i18n'
import { ArrowLeft } from 'lucide-react'

export interface Crumb {
  label: string
  href?: string
}

interface BreadcrumbsProps {
  crumbs: Crumb[]
  className?: string
}

export function getBreadcrumbCopy(t: Translation) {
  return {
    label: t.common?.breadcrumb ?? 'Breadcrumb',
    backHome: t.registry?.backHome ?? 'Back',
  }
}

// Breadcrumb strip rendered in page content (not inside the fixed header),
// so the header stays byte-for-byte identical across homepage and subpages.
// The first segment is always "Home → ..." linking to the language-aware
// landing page.
export default function Breadcrumbs({ crumbs, className }: BreadcrumbsProps) {
  const lang = useAppStore(s => s.lang)
  const t = getTranslation(lang)
  const copy = getBreadcrumbCopy(t)
  const homeHref = lang === 'en' ? '/' : `/${lang}/`
  return (
    <nav aria-label={copy.label} className={cn('flex items-center gap-1.5 text-sm text-gray-500 dark:text-gray-400 min-w-0 overflow-x-auto whitespace-nowrap', className)}>
      <a href={homeHref} className="inline-flex items-center gap-1 hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors shrink-0">
        <ArrowLeft className="w-3.5 h-3.5" />
        {copy.backHome}
      </a>
      {crumbs.map((c, i) => {
        const isLast = i === crumbs.length - 1
        return (
          <span key={`${c.href ?? 'current'}:${c.label}`} className="flex items-center gap-1.5 shrink-0">
            <span aria-hidden="true" className="text-gray-300 dark:text-gray-700 shrink-0">/</span>
            {isLast || !c.href ? (
              <span className={cn(isLast && 'text-slate-900 dark:text-white font-semibold')}>{c.label}</span>
            ) : (
              <a href={c.href} className="hover:text-cyan-600 dark:hover:text-cyan-400 transition-colors">{c.label}</a>
            )}
          </span>
        )
      })}
    </nav>
  )
}
