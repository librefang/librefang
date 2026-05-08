import { setupBundleMode } from "./lib/bundleMode";
// Patch `window.fetch` and `window.WebSocket` BEFORE any module that
// might issue a request — React Query, Router, i18n loaders all run
// during their own imports below. No-op on non-Tauri origins and on
// debug builds, where the dashboard is served same-origin from the
// daemon.
setupBundleMode();

import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { ToastContainer } from "./components/ui/Toast";
import "./index.css";
import i18n from "./lib/i18n";
import { channelKeys, handKeys, mcpKeys, pluginKeys } from "./lib/queries/keys";

interface RootErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

class RootErrorBoundary extends React.Component<
  { children: React.ReactNode },
  RootErrorBoundaryState
> {
  state: RootErrorBoundaryState = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): RootErrorBoundaryState {
    return { hasError: true, error };
  }

  render() {
    if (this.state.hasError) {
      // Inline styles: this boundary may render before CSS is loaded.
      return (
        <div style={{ display: "flex", height: "100vh", alignItems: "center", justifyContent: "center" }}>
          <div style={{ textAlign: "center", padding: "2rem", maxWidth: "32rem" }}>
            <p style={{ fontSize: "1.125rem", fontWeight: 600, marginBottom: "0.5rem" }}>
              Something went wrong
            </p>
            <p style={{ fontSize: "0.875rem", color: "#6b7280", wordBreak: "break-word" }}>
              {this.state.error?.message ?? "An unexpected error occurred."}
            </p>
            <button
              onClick={() => window.location.reload()}
              style={{
                marginTop: "1rem",
                borderRadius: "0.75rem",
                backgroundColor: "#FF6A3D",
                padding: "0.625rem 1.5rem",
                fontSize: "0.875rem",
                fontWeight: 700,
                color: "#fff",
                border: "none",
                cursor: "pointer",
              }}
            >
              Reload
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
      refetchIntervalInBackground: false,
    }
  }
});

// Backend resolves Accept-Language against `[i18n.<lang>]` blocks in
// plugin / MCP catalog / hand / channel manifests, so the response body
// changes when the user flips languages in the UI. React Query keys do
// not encode language, so we invalidate the affected domains on each
// `languageChanged` event to force a refetch with the new header.
const onLanguageChanged = () => {
  for (const all of [pluginKeys.all, mcpKeys.all, handKeys.all, channelKeys.all]) {
    queryClient.invalidateQueries({ queryKey: all });
  }
};
i18n.on("languageChanged", onLanguageChanged);

// Vite HMR re-runs this module on edit, which would stack a fresh listener
// on top of the previous one each time. Detach on dispose so dev sessions
// don't accumulate redundant invalidations.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    i18n.off("languageChanged", onLanguageChanged);
  });
}

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Root element #root not found — cannot mount dashboard.");
}

createRoot(rootEl).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
        <ToastContainer />
      </QueryClientProvider>
    </RootErrorBoundary>
  </React.StrictMode>
);
