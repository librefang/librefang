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

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <ToastContainer />
    </QueryClientProvider>
  </React.StrictMode>
);
