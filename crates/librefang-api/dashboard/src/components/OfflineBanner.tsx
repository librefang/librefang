import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { WifiOff, RefreshCw } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

export function OfflineBanner() {
  const qc = useQueryClient();
  const [offline, setOffline] = useState(false);
  const [retrying, setRetrying] = useState(false);

  useEffect(() => {
    const cache = qc.getQueryCache();
    const unsub = cache.subscribe((event) => {
      if (event.type === "updated") {
        const { status, error } = event.query.state;
        if (status === "error") {
          const err = error as { status?: number } | null;
          // Only surface network / server errors, not expected 4xx responses.
          if (!err?.status || err.status >= 500) {
            setOffline(true);
          }
        } else if (status === "success") {
          setOffline(false);
        }
      }
    });
    return unsub;
  }, [qc]);

  const retry = async () => {
    setRetrying(true);
    try {
      await qc.refetchQueries({ type: "active" });
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
            disabled={retrying}
            className="ml-2 flex items-center gap-1.5 rounded-lg border border-white/30 px-2.5 py-1 text-xs hover:bg-white/10 transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`w-3 h-3 ${retrying ? "animate-spin" : ""}`} />
            Retry
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
