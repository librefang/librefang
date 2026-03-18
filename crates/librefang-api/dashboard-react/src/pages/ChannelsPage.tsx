import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import {
  listChannels,
  reloadChannels,
  testChannel,
  type ApiActionResponse,
  type ChannelItem
} from "../api";

const REFRESH_MS = 30000;

interface ChannelFeedback {
  type: "ok" | "error";
  text: string;
}

function channelStatus(channel: ChannelItem): {
  label: string;
  className: string;
} {
  if (channel.configured && channel.has_token) {
    return {
      label: "Ready",
      className: "border-success/20 bg-success/10 text-success"
    };
  }
  if (channel.configured) {
    return {
      label: "Configured (No Token)",
      className: "border-warning/20 bg-warning/10 text-warning"
    };
  }
  return {
    label: "Not Configured",
    className: "border-border-subtle bg-surface-hover text-text-dim"
  };
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  return JSON.stringify(action);
}

export function ChannelsPage() {
  const [reloadFeedback, setReloadFeedback] = useState<ChannelFeedback | null>(null);
  const [testFeedback, setTestFeedback] = useState<Record<string, ChannelFeedback>>({});
  const [pendingChannelName, setPendingChannelName] = useState<string | null>(null);

  const channelsQuery = useQuery({
    queryKey: ["channels", "list"],
    queryFn: listChannels,
    refetchInterval: REFRESH_MS
  });

  const reloadMutation = useMutation({
    mutationFn: reloadChannels
  });
  const testMutation = useMutation({
    mutationFn: testChannel
  });

  const channels = channelsQuery.data ?? [];
  const readyCount = channels.filter((channel) => channel.configured && channel.has_token).length;
  const channelsError = channelsQuery.error instanceof Error ? channelsQuery.error.message : "";

  async function handleReload() {
    try {
      const result = await reloadMutation.mutateAsync();
      setReloadFeedback({ type: "ok", text: actionText(result) });
      await channelsQuery.refetch();
    } catch (error) {
      setReloadFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Channel reload failed."
      });
    }
  }

  async function handleTest(channelName: string) {
    setPendingChannelName(channelName);
    try {
      const result = await testMutation.mutateAsync(channelName);
      setTestFeedback((current) => ({
        ...current,
        [channelName]: { type: "ok", text: actionText(result) }
      }));
    } catch (error) {
      setTestFeedback((current) => ({
        ...current,
        [channelName]: {
          type: "error",
          text: error instanceof Error ? error.message : "Channel test failed."
        }
      }));
    } finally {
      setPendingChannelName(null);
    }
  }

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <h1 className="m-0 text-3xl font-extrabold tracking-tight">Channels</h1>
          <p className="mt-1 text-sm text-text-dim font-medium">Messaging and delivery connectors.</p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <span className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim">
            {readyCount}/{channels.length} ready
          </span>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
            type="button"
            onClick={() => void channelsQuery.refetch()}
            disabled={channelsQuery.isFetching}
          >
            Refresh
          </button>
          <button
            className="flex h-9 items-center gap-2 rounded-xl bg-brand px-4 text-sm font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50"
            type="button"
            onClick={() => void handleReload()}
            disabled={reloadMutation.isPending}
          >
            {reloadMutation.isPending ? "Reloading..." : "Reload Bridges"}
          </button>
        </div>
      </header>

      {channelsError ? (
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-error font-bold">{channelsError}</div>
      ) : null}

      {reloadFeedback ? (
        <div
          className={`rounded-xl border p-4 text-sm font-bold shadow-sm ${
            reloadFeedback.type === "ok"
              ? "border-success/20 bg-success/5 text-success"
              : "border-error/20 bg-error/5 text-error"
          }`}
        >
          {reloadFeedback.text}
        </div>
      ) : null}

      {channelsQuery.isLoading && channels.length === 0 ? (
        <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-sm text-text-dim font-medium italic shadow-sm">
          Loading channels...
        </div>
      ) : null}

      {!channelsQuery.isLoading && channels.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border-subtle bg-surface p-8 text-center text-sm text-text-dim font-medium shadow-sm">
          No channels found.
        </div>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {channels.map((channel) => {
          const status = channelStatus(channel);
          const feedback = testFeedback[channel.name];
          return (
            <article key={channel.name} className="flex flex-col rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm ring-1 ring-black/5 dark:ring-white/5 transition-all hover:border-brand/30">
              <div className="mb-4 flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="m-0 truncate text-base font-bold tracking-tight">{channel.display_name ?? channel.name}</h2>
                  <p className="text-[10px] font-bold text-text-dim uppercase tracking-widest">{channel.name}</p>
                </div>
                <span className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${status.className}`}>{status.label}</span>
              </div>

              <p className="mb-4 flex-1 text-sm font-medium text-text-dim/80 line-clamp-2">{channel.description ?? "No description."}</p>

              <div className="mb-4 space-y-2">
                <div className="flex justify-between border-b border-border-subtle pb-1">
                  <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Category</span>
                  <span className="text-[11px] font-bold">{channel.category ?? "-"}</span>
                </div>
                <div className="flex justify-between border-b border-border-subtle pb-1">
                  <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Difficulty</span>
                  <span className="text-[11px] font-bold">{channel.difficulty ?? "-"}</span>
                </div>
                <div className="flex justify-between border-b border-border-subtle pb-1">
                  <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Setup time</span>
                  <span className="text-[11px] font-bold">{channel.setup_time ?? "-"}</span>
                </div>
              </div>

              {channel.quick_setup ? (
                <div className="mb-4 rounded-xl border border-border-subtle bg-main/40 p-3 text-xs font-medium text-text-dim italic leading-relaxed">
                  {channel.quick_setup}
                </div>
              ) : null}

              {channel.setup_steps && channel.setup_steps.length > 0 ? (
                <div className="mb-4">
                  <p className="mb-2 text-[10px] font-black uppercase tracking-widest text-text-dim">Setup Steps</p>
                  <ol className="list-inside list-decimal space-y-1.5 text-[11px] font-medium text-text-dim/80">
                    {channel.setup_steps.slice(0, 3).map((step, index) => (
                      <li key={`${channel.name}-step-${index}`} className="leading-snug">{step}</li>
                    ))}
                  </ol>
                </div>
              ) : null}

              <div className="mt-auto pt-4 border-t border-border-subtle/50 flex items-center justify-between">
                <button
                  className="rounded-lg border border-border-subtle bg-surface px-4 py-2 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
                  type="button"
                  onClick={() => void handleTest(channel.name)}
                  disabled={pendingChannelName === channel.name}
                >
                  {pendingChannelName === channel.name ? "Testing..." : "Test Connectivity"}
                </button>
              </div>

              {feedback ? (
                <p
                  className={`mt-4 rounded-xl border p-3 text-xs font-bold ${
                    feedback.type === "ok"
                      ? "border-success/20 bg-success/5 text-success"
                      : "border-error/20 bg-error/5 text-error"
                  }`}
                >
                  {feedback.text}
                </p>
              ) : null}
            </article>
          );
        })}
      </div>
    </section>
  );
}
