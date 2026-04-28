import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { WifiOff, RefreshCw } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useHealthDetail } from "../lib/queries/runtime";
import { runtimeKeys } from "../lib/queries/keys";

/**
 * Surfaces a "daemon unreachable" banner when the dedicated `/api/health/detail`
 * polling query fails. Anchoring on a single connectivity probe avoids the
 * earlier behavior where any 5xx from any unrelated endpoint flickered the
 * banner — daemon status now drives daemon-status UI, nothing else.
 */
export function OfflineBanner() {
  const qc = useQueryClient();
  const { isError, isFetching, refetch } = useHealthDetail();
  const [retrying, setRetrying] = useState(false);

  const offline = isError;

  const retry = async () => {
    setRetrying(true);
    try {
      await refetch();
      // Once connectivity is back, prod any other queries that may have
      // failed during the outage so the rest of the dashboard refreshes.
      await qc.refetchQueries({ queryKey: runtimeKeys.all, exact: false });
    } finally {
      setRetrying(false);
    }
  };

  return (
    <AnimatePresence>
      {offline && (
        <motion.div
          initial={{ y: -40, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: -40, opacity: 0 }}
          transition={{ type: "spring", stiffness: 300, damping: 30 }}
          className="fixed top-0 inset-x-0 z-[60] flex items-center justify-center gap-3 px-4 py-2 bg-error/90 text-white text-sm font-medium backdrop-blur-sm"
        >
          <WifiOff className="w-4 h-4 shrink-0" />
          <span>Daemon unreachable</span>
          <button
            onClick={retry}
            disabled={retrying || isFetching}
            className="ml-2 flex items-center gap-1.5 rounded-lg border border-white/30 px-2.5 py-1 text-xs hover:bg-white/10 transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`w-3 h-3 ${retrying || isFetching ? "animate-spin" : ""}`} />
            Retry
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
