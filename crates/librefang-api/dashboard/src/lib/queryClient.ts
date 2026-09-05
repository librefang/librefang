import { MutationCache, QueryClient } from "@tanstack/react-query";
import i18n from "./i18n";
import { toastErr } from "./errors";
import { useUIStore } from "./store";

function errorToastIds(): Set<string> {
  return new Set(
    useUIStore
      .getState()
      .toasts.filter((toast) => toast.type === "error")
      .map((toast) => toast.id),
  );
}

export function createDashboardQueryClient(): QueryClient {
  // Failures nobody has reported to the user yet, keyed by the failing mutation.
  const unreported = new WeakMap<object, { error: unknown; before: Set<string> }>();

  // Toasts this fallback raised itself.
  // They are not evidence that anyone reported anything: without this set, two unreported failures landing in the same macrotask collapse into a single toast, because the first one's fallback toast looks to the second exactly like a call site having spoken up.
  // ProvidersPage's four-way concurrent `testMutation.mutateAsync` batch and GoalsPage's template batch both produce that shape against hooks that declare no `onError`, and their per-item messages genuinely differ.
  const ownToastIds = new Set<string>();

  const client = new QueryClient({
    mutationCache: new MutationCache({
      onError: (error, _variables, _context, mutation) => {
        // A mutation-specific handler can provide more actionable context and already owns its UI.
        // This callback is the fallback for mutate() calls that would otherwise fail without any visible feedback.
        if (mutation.options.onError) return;

        // `mutation.options` only ever carries the `useMutation` options, so a handler passed per call as `mutate(vars, { onError })` — how nearly every dashboard page supplies its error UI — is invisible from here, and used to earn a second, generic toast stacked on top of the specific one.
        // Those handlers run later: query-core fires them from the mutation's observer during the terminal dispatch, and a `mutateAsync` caller's `catch` runs microtasks later still.
        // So record the failure and let the cache subscription below decide once everyone else has had their turn.
        unreported.set(mutation, { error, before: errorToastIds() });
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

  client.getMutationCache().subscribe((event) => {
    if (event.type !== "updated" || event.action.type !== "error") return;
    const failure = unreported.get(event.mutation);
    if (!failure) return;
    unreported.delete(event.mutation);

    // The cache is notified after its observers, so a `mutate(vars, { onError })` handler has already run by the time this fires; the macrotask hop additionally waits out the microtasks that carry a `mutateAsync` rejection to the call site's `catch`.
    setTimeout(() => {
      const toasts = useUIStore.getState().toasts;
      const liveIds = new Set(toasts.map((toast) => toast.id));
      // Ids leave the store when the user dismisses a toast or MAX_TOASTS evicts it; drop them here too, so this set stays bounded by the store rather than by session length.
      for (const id of ownToastIds) {
        if (!liveIds.has(id)) ownToastIds.delete(id);
      }

      // "Someone else told the user" means an error toast that was not on screen when this failure happened and did not come from this fallback.
      // The residual imprecision is that any such toast counts, including one raised by an unrelated failure inside the same macrotask; the fallback stays quiet in that case rather than risk the duplicate it exists to avoid.
      const reportedElsewhere = toasts.some(
        (toast) =>
          toast.type === "error" && !failure.before.has(toast.id) && !ownToastIds.has(toast.id),
      );
      if (reportedElsewhere) return;

      const fallback = i18n.t("common.error", { defaultValue: "Error" });
      useUIStore.getState().addToast(toastErr(failure.error, fallback), "error");
      for (const toast of useUIStore.getState().toasts) {
        if (!liveIds.has(toast.id)) ownToastIds.add(toast.id);
      }
    }, 0);
  });

  return client;
}
