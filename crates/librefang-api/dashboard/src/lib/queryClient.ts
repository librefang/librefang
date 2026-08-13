import { MutationCache, QueryClient } from "@tanstack/react-query";
import i18n from "./i18n";
import { toastErr } from "./errors";
import { useUIStore } from "./store";

export function createDashboardQueryClient(): QueryClient {
  return new QueryClient({
    mutationCache: new MutationCache({
      onError: (error, _variables, _context, mutation) => {
        // A mutation-specific handler can provide more actionable context and
        // already owns its UI. This callback is the fallback for bare mutate()
        // calls that would otherwise fail without any visible feedback.
        if (mutation.options.onError) return;

        const fallback = i18n.t("common.error", { defaultValue: "Error" });
        useUIStore.getState().addToast(toastErr(error, fallback), "error");
      },
    }),
    defaultOptions: {
      queries: {
        retry: 1,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
        refetchIntervalInBackground: false,
      },
    },
  });
}
