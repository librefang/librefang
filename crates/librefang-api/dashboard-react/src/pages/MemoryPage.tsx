import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useEffect, useState } from "react";
import {
  addMemoryFromText,
  cleanupMemories,
  decayMemories,
  deleteMemory,
  getMemoryStats,
  listAgents,
  listMemories,
  searchMemories,
  type MemoryItem
} from "../api";

const REFRESH_MS = 30000;
const PAGE_SIZE = 20;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function dateText(value?: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function levelText(level?: string): string {
  if (!level) return "session";
  return level.toLowerCase();
}

function levelClass(level?: string): string {
  const normalized = levelText(level);
  if (normalized.includes("user")) return "border-emerald-700 bg-emerald-700/15 text-emerald-100";
  if (normalized.includes("agent")) return "border-sky-700 bg-sky-700/15 text-sky-100";
  return "border-amber-700 bg-amber-700/15 text-amber-100";
}

export function MemoryPage() {
  const queryClient = useQueryClient();
  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [searchInput, setSearchInput] = useState("");
  const [searchMode, setSearchMode] = useState(false);
  const [offset, setOffset] = useState(0);
  const [addContent, setAddContent] = useState("");
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const agentsQuery = useQuery({
    queryKey: ["agents", "list", "memory-helper"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const statsQuery = useQuery({
    queryKey: ["memory", "stats", selectedAgentId],
    queryFn: () => getMemoryStats(selectedAgentId || undefined),
    refetchInterval: REFRESH_MS
  });

  const listQuery = useQuery({
    queryKey: ["memory", "list", selectedAgentId, offset, PAGE_SIZE],
    queryFn: () =>
      listMemories({
        agentId: selectedAgentId || undefined,
        offset,
        limit: PAGE_SIZE
      }),
    enabled: !searchMode,
    refetchInterval: REFRESH_MS
  });

  const searchQuery = useQuery({
    queryKey: ["memory", "search", selectedAgentId, searchInput],
    queryFn: () =>
      searchMemories({
        query: searchInput.trim(),
        agentId: selectedAgentId || undefined,
        limit: 100
      }),
    enabled: searchMode && searchInput.trim().length > 0
  });

  const addMutation = useMutation({
    mutationFn: ({ content, agentId }: { content: string; agentId?: string }) =>
      addMemoryFromText(content, agentId)
  });
  const deleteMutation = useMutation({
    mutationFn: deleteMemory
  });
  const cleanupMutation = useMutation({
    mutationFn: cleanupMemories
  });
  const decayMutation = useMutation({
    mutationFn: decayMemories
  });

  useEffect(() => {
    setOffset(0);
    setSearchMode(false);
  }, [selectedAgentId]);

  const memories = (searchMode ? searchQuery.data : listQuery.data?.memories) ?? [];
  const total = searchMode ? memories.length : listQuery.data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil((total || 0) / PAGE_SIZE));
  const currentPage = Math.floor(offset / PAGE_SIZE) + 1;

  const stats = statsQuery.data;
  const agents = agentsQuery.data ?? [];
  const error = (() => {
    if (statsQuery.error instanceof Error) return statsQuery.error.message;
    if (listQuery.error instanceof Error) return listQuery.error.message;
    if (searchQuery.error instanceof Error) return searchQuery.error.message;
    return "";
  })();

  async function refreshAll() {
    await queryClient.invalidateQueries({ queryKey: ["memory"] });
    await Promise.all([statsQuery.refetch(), listQuery.refetch(), searchQuery.refetch()]);
  }

  async function handleSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = searchInput.trim();
    if (!text) {
      setSearchMode(false);
      return;
    }
    setSearchMode(true);
    await searchQuery.refetch();
  }

  async function handleAddMemory(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = addContent.trim();
    if (!text || addMutation.isPending) return;

    try {
      await addMutation.mutateAsync({
        content: text,
        agentId: selectedAgentId || undefined
      });
      setFeedback({ type: "ok", text: "Memory added." });
      setAddContent("");
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to add memory."
      });
    }
  }

  async function handleDelete(item: MemoryItem) {
    if (deleteMutation.isPending) return;
    if (!window.confirm("Delete this memory?")) return;

    setPendingDeleteId(item.id);
    try {
      await deleteMutation.mutateAsync(item.id);
      setFeedback({ type: "ok", text: "Memory deleted." });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Failed to delete memory."
      });
    } finally {
      setPendingDeleteId(null);
    }
  }

  async function handleCleanup() {
    if (cleanupMutation.isPending) return;
    if (!window.confirm("Remove expired session memories now?")) return;
    try {
      await cleanupMutation.mutateAsync();
      setFeedback({ type: "ok", text: "Memory cleanup completed." });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Cleanup failed."
      });
    }
  }

  async function handleDecay() {
    if (decayMutation.isPending) return;
    if (!window.confirm("Apply confidence decay now?")) return;
    try {
      await decayMutation.mutateAsync();
      setFeedback({ type: "ok", text: "Confidence decay completed." });
      await refreshAll();
    } catch (mutationError) {
      setFeedback({
        type: "error",
        text: mutationError instanceof Error ? mutationError.message : "Decay failed."
      });
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Memory</h1>
          <p className="text-sm text-slate-400">Proactive memory inventory with search, maintenance, and manual add.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            onClick={() => void refreshAll()}
            disabled={statsQuery.isFetching || listQuery.isFetching || searchQuery.isFetching}
          >
            Refresh
          </button>
          <button
            className="rounded-lg border border-amber-700 bg-amber-700/10 px-3 py-2 text-sm font-medium text-amber-200 transition hover:bg-amber-700/20 disabled:cursor-not-allowed disabled:opacity-60"
            onClick={() => void handleCleanup()}
            disabled={cleanupMutation.isPending}
          >
            Cleanup
          </button>
          <button
            className="rounded-lg border border-violet-700 bg-violet-700/10 px-3 py-2 text-sm font-medium text-violet-200 transition hover:bg-violet-700/20 disabled:cursor-not-allowed disabled:opacity-60"
            onClick={() => void handleDecay()}
            disabled={decayMutation.isPending}
          >
            Decay
          </button>
        </div>
      </header>

      {feedback ? (
        <div
          className={`rounded-xl border p-3 text-sm ${
            feedback.type === "ok"
              ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
              : "border-rose-700 bg-rose-700/10 text-rose-200"
          }`}
        >
          {feedback.text}
        </div>
      ) : null}
      {error ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{error}</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Total</span>
          <strong className="mt-1 block text-2xl">{stats?.total ?? 0}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">User Level</span>
          <strong className="mt-1 block text-2xl">{stats?.user_count ?? 0}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Session Level</span>
          <strong className="mt-1 block text-2xl">{stats?.session_count ?? 0}</strong>
        </article>
        <article className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <span className="text-sm text-slate-400">Agent Level</span>
          <strong className="mt-1 block text-2xl">{stats?.agent_count ?? 0}</strong>
        </article>
      </div>

      <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
        <div className="grid gap-2 lg:grid-cols-[260px_1fr_auto]">
          <select
            value={selectedAgentId}
            onChange={(event) => setSelectedAgentId(event.target.value)}
            className="rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
          >
            <option value="">All agents</option>
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
          <form className="flex gap-2" onSubmit={handleSearch}>
            <input
              value={searchInput}
              onChange={(event) => setSearchInput(event.target.value)}
              placeholder="Search memories"
              className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
            />
            <button
              type="submit"
              className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500"
            >
              Search
            </button>
          </form>
          {searchMode ? (
            <button
              className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-slate-400 hover:bg-slate-700"
              onClick={() => setSearchMode(false)}
            >
              Clear
            </button>
          ) : null}
        </div>

        <form className="mt-3 flex gap-2" onSubmit={handleAddMemory}>
          <input
            value={addContent}
            onChange={(event) => setAddContent(event.target.value)}
            placeholder="Add memory content..."
            className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
          />
          <button
            type="submit"
            className="rounded-lg border border-emerald-600 bg-emerald-700/20 px-3 py-2 text-sm font-medium text-emerald-200 transition hover:bg-emerald-700/30 disabled:cursor-not-allowed disabled:opacity-60"
            disabled={addMutation.isPending || addContent.trim().length === 0}
          >
            Add
          </button>
        </form>

        <div className="mt-3 text-xs text-slate-400">
          {searchMode ? "Search results" : `Page ${currentPage}/${totalPages}`} · {memories.length} items
        </div>

        {listQuery.isLoading || searchQuery.isLoading ? (
          <p className="mt-3 text-sm text-slate-400">Loading memories...</p>
        ) : memories.length === 0 ? (
          <p className="mt-3 text-sm text-slate-400">No memories found.</p>
        ) : (
          <ul className="mt-3 flex max-h-[520px] list-none flex-col gap-2 overflow-y-auto p-0">
            {memories.map((item) => (
              <li key={item.id} className="rounded-lg border border-slate-800 bg-slate-950/70 p-3">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <span className={`rounded-full border px-2 py-1 text-xs ${levelClass(item.level)}`}>
                    {levelText(item.level)}
                  </span>
                  <button
                    className="rounded-lg border border-rose-700 bg-rose-700/10 px-2 py-1 text-xs text-rose-200 transition hover:bg-rose-700/20 disabled:cursor-not-allowed disabled:opacity-60"
                    onClick={() => void handleDelete(item)}
                    disabled={pendingDeleteId === item.id}
                  >
                    Delete
                  </button>
                </div>
                <p className="m-0 mt-2 whitespace-pre-wrap break-words text-sm">{item.content ?? "-"}</p>
                <p className="m-0 mt-2 text-xs text-slate-500">
                  {item.category ? `${item.category} · ` : ""}
                  {dateText(item.created_at)}
                </p>
              </li>
            ))}
          </ul>
        )}

        {!searchMode ? (
          <div className="mt-3 flex items-center justify-between">
            <button
              className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-xs font-medium text-slate-100 transition hover:border-slate-400 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => setOffset((current) => Math.max(0, current - PAGE_SIZE))}
              disabled={offset === 0}
            >
              Previous
            </button>
            <button
              className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-xs font-medium text-slate-100 transition hover:border-slate-400 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => setOffset((current) => current + PAGE_SIZE)}
              disabled={currentPage >= totalPages}
            >
              Next
            </button>
          </div>
        ) : null}
      </div>
    </section>
  );
}
