import { QueryClient } from '@tanstack/react-query'

export const DEFAULT_QUERY_STALE_TIME_MS = 30_000

export function createAppQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: DEFAULT_QUERY_STALE_TIME_MS,
        refetchOnWindowFocus: true,
      },
    },
  })
}

export function requireRootElement(document: Document): HTMLElement {
  const rootElement = document.getElementById('root')
  if (!rootElement) {
    throw new Error('Root element #root not found; ensure the HTML template includes <div id="root"></div>')
  }
  return rootElement
}
