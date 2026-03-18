import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { listMemories, deleteMemory, type MemoryItem } from "../api";

const REFRESH_MS = 30000;

export function MemoryPage() {
  const queryClient = useQueryClient();
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const memoryQuery = useQuery({
    queryKey: ["memory", "list"],
    queryFn: listMemories,
    refetchInterval: REFRESH_MS
  });

  const deleteMutation = useMutation({
    mutationFn: deleteMemory
  });

  const memories = memoryQuery.data?.memories ?? [];
  const totalCount = memoryQuery.data?.total ?? 0;

  async function handleDelete(id: string) {
    if (!window.confirm("Are you sure you want to delete this memory?")) return;
    setPendingDeleteId(id);
    try {
      await deleteMutation.mutateAsync(id);
      await queryClient.invalidateQueries({ queryKey: ["memory"] });
    } catch (e) {
      console.error("Delete failed", e);
    } finally {
      setPendingDeleteId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            </svg>
            Cognitive Layer
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Memory</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Persistent vector storage and episodic recall for long-term agent context.</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim sm:block">
            {totalCount} Objects Stored
          </div>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand transition-all shadow-sm"
            onClick={() => void memoryQuery.refetch()}
          >
            <svg className={`h-3.5 w-3.5 ${memoryQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Refresh
          </button>
        </div>
      </header>

      <div className="grid gap-4">
        {memories.map((m: MemoryItem) => (
          <article key={m.id} className="group rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm transition-all hover:border-brand/30 ring-1 ring-black/5 dark:ring-white/5">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 mb-1">
                  <h2 className="text-sm font-black truncate">{m.id}</h2>
                  <span className="rounded-lg bg-brand/10 border border-brand/10 px-2 py-0.5 text-[9px] font-black text-brand uppercase tracking-tighter">Vector</span>
                </div>
                <p className="text-xs text-text-dim line-clamp-2 leading-relaxed mb-3">{m.content || "No content summary available."}</p>
                <div className="flex flex-wrap gap-3">
                  <div className="flex items-center gap-1 text-[10px] font-bold text-text-dim/60 uppercase">
                    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M12 8v4l3 2" /></svg>
                    {new Date(m.created_at || "").toLocaleDateString()}
                  </div>
                  {m.metadata && (
                    <div className="flex items-center gap-1 text-[10px] font-bold text-brand uppercase">
                      <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path d="M7 7h.01M7 3h10a2 2 0 012 2v14a2 2 0 01-2 2H7a2 2 0 01-2-2V5a2 2 0 012-2z" /></svg>
                      {Object.keys(m.metadata).length} meta keys
                    </div>
                  )}
                </div>
              </div>
              <button
                onClick={() => void handleDelete(m.id)}
                disabled={pendingDeleteId === m.id}
                className="opacity-0 group-hover:opacity-100 transition-opacity p-2 rounded-lg text-text-dim hover:text-error hover:bg-error/5"
              >
                <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
                  <path strokeLinecap="round" strokeLinejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
              </button>
            </div>
          </article>
        ))}

        {memories.length === 0 && !memoryQuery.isLoading && (
          <div className="py-24 text-center border border-dashed border-border-subtle rounded-2xl bg-surface/30">
            <p className="text-sm text-text-dim font-bold tracking-tight">No memory clusters found.</p>
          </div>
        )}
      </div>
    </div>
  );
}
