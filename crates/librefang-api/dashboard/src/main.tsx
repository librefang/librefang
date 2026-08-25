import { setupBundleMode } from "./lib/bundleMode";
// Patch `window.fetch` and `window.WebSocket` BEFORE any module that
// might issue a request — React Query, Router, i18n loaders all run
// during their own imports below. No-op on non-Tauri origins and on
// debug builds, where the dashboard is served same-origin from the
// daemon.
setupBundleMode();

import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { ToastContainer } from "./components/ui/Toast";
import { RootErrorBoundary } from "./components/RootErrorBoundary";
import "./index.css";
import i18n, { i18nReady } from "./lib/i18n";
import { createDashboardQueryClient } from "./lib/queryClient";

const queryClient = createDashboardQueryClient();

// Every authenticated request carries Accept-Language. Several manifest
// domains return localized bodies, and that set grows over time. Query keys do
// not encode language, so invalidate the whole cache instead of maintaining a
// fragile domain allowlist whenever the user explicitly changes language.
const onLanguageChanged = () => {
  void queryClient.invalidateQueries();
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

const mountDashboard = async (): Promise<void> => {
  await i18nReady;
  createRoot(rootEl).render(
    <React.StrictMode>
      <RootErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
          <ToastContainer />
        </QueryClientProvider>
      </RootErrorBoundary>
    </React.StrictMode>,
  );
};

void mountDashboard().catch((error: unknown) => {
  console.error("[dashboard] failed to mount", error);
});
