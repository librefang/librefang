import { useQuery } from '@tanstack/react-query'
import { Loader2, BarChart3 } from 'lucide-react'
import { useAppStore } from '../store'
import { translations } from '../i18n'
import SubpageHeader from '../components/SubpageHeader'

const METRICS_API = 'https://stats.librefang.ai/api/registry/metrics'

interface Metrics {
  generatedAt: string
  perCategory: Record<string, { total: number; items: number }>
  topOverall: { category: string; id: string; clicks: number }[]
  totalClicks: number
}

async function fetchMetrics(): Promise<Metrics> {
  const res = await fetch(METRICS_API)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return res.json()
}

interface MetricsPageProps {
  onOpenSearch?: () => void
}

export default function MetricsPage({ onOpenSearch }: MetricsPageProps) {
  const lang = useAppStore(s => s.lang)
  // `translations[lang]` kept for future copy; metrics page is currently
  // English-first (content is numeric / CLI-style).
  void translations[lang]
  const { data, isLoading, error } = useQuery<Metrics>({
    queryKey: ['registry-metrics'],
    queryFn: fetchMetrics,
    staleTime: 1000 * 60 * 5,
    retry: 1,
  })
  const langPrefix = lang === 'en' ? '' : `/${lang}`

  return (
    <main className="min-h-screen bg-surface">
      <SubpageHeader
        crumbs={[{ label: 'Metrics' }]}
        onOpenSearch={onOpenSearch}
      />

      <section className="max-w-5xl mx-auto px-6 py-14">
        <div className="mb-8">
          <div className="text-xs font-mono text-cyan-600 dark:text-cyan-500 uppercase tracking-widest mb-3 flex items-center gap-2">
            <BarChart3 className="w-3.5 h-3.5" />
            Registry Metrics
          </div>
          <h1 className="text-3xl md:text-5xl font-black text-slate-900 dark:text-white tracking-tight mb-2">
            Click telemetry
          </h1>
          <p className="text-gray-600 dark:text-gray-400 text-base">
            Aggregate counts of detail-page views across the registry, recorded via
            the <code className="text-cyan-600 dark:text-cyan-400">/api/registry/click</code> worker endpoint.
          </p>
        </div>

        {isLoading && (
          <div className="flex items-center justify-center py-16 text-gray-400">
            <Loader2 className="w-5 h-5 animate-spin mr-2" />
          </div>
        )}

        {error && !isLoading && (
          <div className="text-sm text-red-400">
            Could not load metrics: {(error as Error).message}
          </div>
        )}

        {data && (
          <>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-10">
              <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-5">
                <div className="text-3xl font-black text-slate-900 dark:text-white font-mono">{data.totalClicks}</div>
                <div className="text-xs text-gray-500 uppercase tracking-wider mt-1">Total clicks</div>
              </div>
              <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-5">
                <div className="text-3xl font-black text-slate-900 dark:text-white font-mono">
                  {Object.values(data.perCategory).reduce((s, c) => s + c.items, 0)}
                </div>
                <div className="text-xs text-gray-500 uppercase tracking-wider mt-1">Unique items</div>
              </div>
              <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-5">
                <div className="text-3xl font-black text-slate-900 dark:text-white font-mono">
                  {Object.keys(data.perCategory).filter(k => data.perCategory[k]!.total > 0).length}
                </div>
                <div className="text-xs text-gray-500 uppercase tracking-wider mt-1">Active categories</div>
              </div>
              <div className="bg-surface-100 border border-black/10 dark:border-white/5 p-5">
                <div className="text-sm font-mono text-slate-900 dark:text-white">
                  {new Date(data.generatedAt).toLocaleString()}
                </div>
                <div className="text-xs text-gray-500 uppercase tracking-wider mt-1">Generated</div>
              </div>
            </div>

            <div className="mb-10">
              <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-4">Per category</h2>
              <div className="border border-black/10 dark:border-white/5 divide-y divide-black/10 dark:divide-white/5">
                {Object.entries(data.perCategory)
                  .sort(([, a], [, b]) => b.total - a.total)
                  .map(([cat, stats]) => (
                    <a
                      key={cat}
                      href={`${langPrefix}/${cat}`}
                      className="flex items-center justify-between px-5 py-3 bg-surface-100 hover:bg-surface-200 transition-colors group"
                    >
                      <span className="font-mono text-sm font-semibold text-slate-900 dark:text-white group-hover:text-cyan-600 dark:group-hover:text-cyan-400">
                        {cat}
                      </span>
                      <div className="flex items-center gap-4 text-xs font-mono text-gray-500">
                        <span>{stats.items} items</span>
                        <span className="text-amber-500">{stats.total} clicks</span>
                      </div>
                    </a>
                  ))}
              </div>
            </div>

            {data.topOverall.length > 0 && (
              <div>
                <h2 className="text-xs font-mono text-gray-500 uppercase tracking-widest mb-4">Top items overall</h2>
                <div className="border border-black/10 dark:border-white/5 divide-y divide-black/10 dark:divide-white/5">
                  {data.topOverall.map(item => (
                    <a
                      key={`${item.category}:${item.id}`}
                      href={`${langPrefix}/${item.category}/${item.id}`}
                      className="flex items-center justify-between px-5 py-3 bg-surface-100 hover:bg-surface-200 transition-colors"
                    >
                      <div className="flex items-center gap-3 min-w-0">
                        <span className="text-[10px] font-mono text-gray-400 dark:text-gray-600 uppercase tracking-wider shrink-0 w-20">
                          {item.category}
                        </span>
                        <span className="font-mono text-sm text-slate-900 dark:text-white truncate">
                          {item.id}
                        </span>
                      </div>
                      <span className="text-xs font-mono text-amber-500 font-bold shrink-0">
                        {item.clicks}
                      </span>
                    </a>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </section>
    </main>
  )
}
