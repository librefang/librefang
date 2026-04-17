import { Component } from 'react'
import type { ReactNode, ErrorInfo } from 'react'

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
}

// Top-level error boundary. If any descendant throws (including lazy-loaded
// routes failing to resolve), we render a minimal recovery card instead of
// a blank white screen. Using a class component because React hooks still
// have no equivalent API for error boundaries.
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error('LibreFang UI error:', error, info.componentStack)
  }

  handleReload = () => {
    this.setState({ error: null })
    window.location.reload()
  }

  render() {
    const err = this.state.error
    if (!err) return this.props.children
    return (
      <div className="min-h-screen flex items-center justify-center p-6">
        <div className="max-w-md w-full border border-red-500/20 bg-red-500/5 p-6 rounded">
          <div className="text-xs font-mono uppercase tracking-widest text-red-400 mb-2">Runtime error</div>
          <h1 className="text-lg font-bold text-slate-900 dark:text-white mb-3">Something went wrong.</h1>
          <pre className="text-xs font-mono text-gray-500 mb-4 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
            {err.message}
          </pre>
          <div className="flex gap-2">
            <button
              onClick={this.handleReload}
              className="px-3 py-1.5 text-xs font-semibold bg-cyan-500 hover:bg-cyan-400 text-surface rounded"
            >
              Reload
            </button>
            <a
              href="/"
              className="px-3 py-1.5 text-xs font-semibold border border-black/10 dark:border-white/10 hover:border-cyan-500/30 rounded text-gray-700 dark:text-gray-300"
            >
              Home
            </a>
          </div>
        </div>
      </div>
    )
  }
}
