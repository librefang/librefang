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
i18n.on("languageChanged", () => {
  for (const all of [pluginKeys.all, mcpKeys.all, handKeys.all, channelKeys.all]) {
    queryClient.invalidateQueries({ queryKey: all });
  }
});

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <ToastContainer />
    </QueryClientProvider>
  </React.StrictMode>
);
