"use client";

import * as React from "react";

interface ErrorBoundaryState {
	hasError: boolean;
	error?: Error;
	errorInfo?: React.ErrorInfo;
}

interface ErrorBoundaryProps {
	children: React.ReactNode;
	fallback?: React.ComponentType<ErrorFallbackProps>;
	onError?: (error: Error, errorInfo: React.ErrorInfo) => void;
}

interface ErrorFallbackProps {
	error?: Error;
	resetError?: () => void;
}

// Error fallback component for the docs site
function DocsErrorFallback({ error, resetError }: ErrorFallbackProps) {
	return (
		<div className="flex min-h-[60vh] flex-col items-center justify-center p-8">
			<div className="text-center">
				<div className="mb-6 text-8xl"></div>
				<h1 className="mb-3 text-3xl font-bold text-zinc-900 dark:text-white">
					Page Failed to Load
				</h1>
				<p className="mb-8 max-w-md text-lg text-zinc-600 dark:text-zinc-400">
					The docs page encountered a problem. This may be an MDX parsing error or a component rendering issue.
				</p>

				{process.env.NODE_ENV === "development" && error && (
					<div className="mb-8 max-w-2xl rounded-lg bg-red-50 p-4 dark:bg-red-950/20">
						<h3 className="mb-2 font-semibold text-red-800 dark:text-red-200">
							Development Mode Error Info
						</h3>
						<details className="text-left">
							<summary className="cursor-pointer text-sm text-red-600 dark:text-red-400">
								Click to view error details
							</summary>
							<pre className="mt-2 overflow-auto text-xs text-red-600 dark:text-red-400">
								{error.name}: {error.message}
								{error.stack}
							</pre>
						</details>
					</div>
				)}

				<div className="flex flex-col gap-3 sm:flex-row sm:justify-center">
					{resetError && (
						<button
							type="button"
							onClick={resetError}
							className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
						>
							Try Again
						</button>
					)}
					<button
						type="button"
						onClick={() => {
							window.location.href = "/";
						}}
						className="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-800"
					>
						Back to Home
					</button>
					<button
						type="button"
						onClick={() => window.location.reload()}
						className="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-800"
					>
						Refresh Page
					</button>
				</div>
			</div>
		</div>
	);
}

// MDX error boundary component
export class MDXErrorBoundary extends React.Component<
	ErrorBoundaryProps,
	ErrorBoundaryState
> {
	constructor(props: ErrorBoundaryProps) {
		super(props);
		this.state = { hasError: false };
	}

	static getDerivedStateFromError(error: Error): ErrorBoundaryState {
		return {
			hasError: true,
			error,
		};
	}

	override componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
		console.error("MDX ErrorBoundary caught an error:", error, errorInfo);

		// Log MDX-related errors
		this.reportMDXError(error, errorInfo);

		this.props.onError?.(error, errorInfo);
	}

	reportMDXError = (error: Error, errorInfo: React.ErrorInfo) => {
		const errorReport = {
			type: "mdx_error",
			message: error.message,
			stack: error.stack,
			componentStack: errorInfo.componentStack,
			timestamp: new Date().toISOString(),
			userAgent:
				typeof window !== "undefined" ? navigator.userAgent : undefined,
			url: typeof window !== "undefined" ? window.location.href : undefined,

			// MDX-specific information
			isDevelopment: process.env.NODE_ENV === "development",
			errorType: this.classifyMDXError(error),
		};

		// Verbose output in development
		if (process.env.NODE_ENV === "development") {
			console.group(" MDX Error Details");
			console.error("Error:", error);
			console.error("Component Stack:", errorInfo.componentStack);
			console.error("Error Type:", errorReport.errorType);
			console.groupEnd();
		}

		// Send error report in production
		if (
			typeof window !== "undefined" &&
			process.env.NODE_ENV === "production"
		) {
			fetch("/api/errors", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(errorReport),
			}).catch(console.error);
		}
	};

	classifyMDXError = (error: Error): string => {
		const message = error.message.toLowerCase();

		if (message.includes("mdx") || message.includes("markdown")) {
			return "mdx_parsing";
		}
		if (message.includes("component") || message.includes("element")) {
			return "component_rendering";
		}
		if (message.includes("import") || message.includes("module")) {
			return "module_import";
		}
		if (message.includes("hook") || message.includes("state")) {
			return "react_hook";
		}

		return "unknown";
	};

	resetError = () => {
		this.setState({ hasError: false, error: undefined, errorInfo: undefined });
	};

	override render() {
		if (this.state.hasError) {
			const FallbackComponent = this.props.fallback || DocsErrorFallback;

			return (
				<FallbackComponent
					error={this.state.error}
					resetError={this.resetError}
				/>
			);
		}

		return this.props.children;
	}
}

// MDX component wrapper for catching errors in individual components
export function MDXComponentWrapper({
	children,
	componentName,
}: {
	children: React.ReactNode;
	componentName?: string;
}) {
	return (
		<MDXErrorBoundary
			fallback={({ error, resetError }) => (
				<div className="my-4 rounded-lg border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950/20">
					<h4 className="mb-2 font-semibold text-red-800 dark:text-red-200">
						Component Render Error {componentName && `(${componentName})`}
					</h4>
					<p className="mb-3 text-sm text-red-600 dark:text-red-400">
						This component could not render properly. This may be due to incorrect props or a component code issue.
					</p>
					{process.env.NODE_ENV === "development" && error && (
						<details className="mb-3">
							<summary className="cursor-pointer text-sm text-red-600 dark:text-red-400">
								View error details
							</summary>
							<pre className="mt-1 text-xs text-red-500">{error.message}</pre>
						</details>
					)}
					<button
						type="button"
						onClick={resetError}
						className="text-sm text-red-600 underline dark:text-red-400"
					>
						Retry
					</button>
				</div>
			)}
		>
			{children}
		</MDXErrorBoundary>
	);
}

// Search error handling
export function useSearchErrorHandler() {
	const [error, setError] = React.useState<string | null>(null);

	const handleSearchError = React.useCallback((error: Error) => {
		console.error("Search error:", error);
		setError("Search is temporarily unavailable. Please try again later.");

		// Auto-clear the error after 5 seconds
		setTimeout(() => setError(null), 5000);
	}, []);

	const clearError = React.useCallback(() => {
		setError(null);
	}, []);

	return { searchError: error, handleSearchError, clearError };
}

// Navigation error handling
export function useNavigationErrorHandler() {
	const handleNavigationError = React.useCallback(
		(href: string, error: Error) => {
			console.error(`Navigation error for ${href}:`, error);

			// Fall back to native navigation
			try {
				window.location.href = href;
			} catch (fallbackError) {
				console.error("Fallback navigation also failed:", fallbackError);
				alert("Navigation failed. Please check that the link is correct.");
			}
		},
		[],
	);

	return { handleNavigationError };
}

// Content loading error handling
export function useContentErrorHandler() {
	const [loadingErrors, setLoadingErrors] = React.useState<Set<string>>(
		new Set(),
	);

	const handleContentError = React.useCallback(
		(contentId: string, error: Error) => {
			console.error(`Content loading error for ${contentId}:`, error);
			setLoadingErrors((prev) => new Set(prev).add(contentId));
		},
		[],
	);

	const retryContent = React.useCallback((contentId: string) => {
		setLoadingErrors((prev) => {
			const newSet = new Set(prev);
			newSet.delete(contentId);
			return newSet;
		});
	}, []);

	const hasError = React.useCallback(
		(contentId: string) => {
			return loadingErrors.has(contentId);
		},
		[loadingErrors],
	);

	return { handleContentError, retryContent, hasError };
}

// Global error handler
export const globalErrorHandler = {
	// Handle uncaught Promise rejections
	setupGlobalHandlers: () => {
		if (typeof window !== "undefined") {
			window.addEventListener("unhandledrejection", (event) => {
				console.error("Unhandled promise rejection:", event.reason);

				// Send error report
				if (process.env.NODE_ENV === "production") {
					fetch("/api/errors", {
						method: "POST",
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify({
							type: "unhandled_rejection",
							reason: event.reason?.toString(),
							timestamp: new Date().toISOString(),
							url: window.location.href,
						}),
					}).catch(console.error);
				}
			});

			// Handle global errors
			window.addEventListener("error", (event) => {
				console.error("Global error:", event.error);

				if (process.env.NODE_ENV === "production") {
					fetch("/api/errors", {
						method: "POST",
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify({
							type: "global_error",
							message: event.message,
							filename: event.filename,
							lineno: event.lineno,
							colno: event.colno,
							error: event.error?.toString(),
							timestamp: new Date().toISOString(),
							url: window.location.href,
						}),
					}).catch(console.error);
				}
			});
		}
	},
};

// Default export for the main error boundary component
export { MDXErrorBoundary as ErrorBoundary };
