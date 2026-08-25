import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import './index.css'
import App from './App'
import ErrorBoundary from './components/ErrorBoundary'
import { createAppQueryClient, requireRootElement } from './lib/bootstrap'

const queryClient = createAppQueryClient()
const rootElement = requireRootElement(document)

createRoot(rootElement).render(
  <StrictMode>
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </ErrorBoundary>
  </StrictMode>,
)
