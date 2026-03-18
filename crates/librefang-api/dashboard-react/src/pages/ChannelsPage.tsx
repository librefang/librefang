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
      className: "border-emerald-700 bg-emerald-700/20 text-emerald-300"
    };
  }
  if (channel.configured) {
    return {
      label: "Configured (No Token)",
      className: "border-amber-700 bg-amber-700/20 text-amber-300"
    };
  }
  return {
    label: "Not Configured",
    className: "border-slate-700 bg-slate-800/60 text-slate-300"
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
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Channels</h1>
          <p className="text-sm text-slate-400">Messaging and delivery connectors.</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300">
            {readyCount}/{channels.length} ready
          </span>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void channelsQuery.refetch()}
            disabled={channelsQuery.isFetching}
          >
            Refresh
          </button>
          <button
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void handleReload()}
            disabled={reloadMutation.isPending}
          >
            {reloadMutation.isPending ? "Reloading..." : "Reload Bridges"}
          </button>
        </div>
      </header>

      {channelsError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{channelsError}</div>
      ) : null}

      {reloadFeedback ? (
        <div
          className={`rounded-xl border p-3 text-sm ${
            reloadFeedback.type === "ok"
              ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
              : "border-rose-700 bg-rose-700/10 text-rose-200"
          }`}
        >
          {reloadFeedback.text}
        </div>
      ) : null}

      {channelsQuery.isLoading && channels.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">Loading channels...</div>
      ) : null}

      {!channelsQuery.isLoading && channels.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">No channels found.</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {channels.map((channel) => {
          const status = channelStatus(channel);
          const feedback = testFeedback[channel.name];
          return (
            <article key={channel.name} className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <div className="mb-3 flex items-start justify-between gap-3">
                <div>
                  <h2 className="m-0 text-base font-semibold">{channel.display_name ?? channel.name}</h2>
                  <p className="text-xs text-slate-500">{channel.name}</p>
                </div>
                <span className={`rounded-full border px-2 py-1 text-[11px] ${status.className}`}>{status.label}</span>
              </div>

              <p className="mb-2 text-sm text-slate-300">{channel.description ?? "No description."}</p>

              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
                <dt className="text-slate-400">Category</dt>
                <dd>{channel.category ?? "-"}</dd>
                <dt className="text-slate-400">Difficulty</dt>
                <dd>{channel.difficulty ?? "-"}</dd>
                <dt className="text-slate-400">Setup time</dt>
                <dd>{channel.setup_time ?? "-"}</dd>
              </dl>

              {channel.quick_setup ? (
                <p className="mt-3 rounded-lg border border-slate-700 bg-slate-950/70 p-2 text-xs text-slate-300">
                  {channel.quick_setup}
                </p>
              ) : null}

              {channel.setup_steps && channel.setup_steps.length > 0 ? (
                <ol className="mt-3 list-inside list-decimal space-y-1 text-xs text-slate-400">
                  {channel.setup_steps.slice(0, 3).map((step, index) => (
                    <li key={`${channel.name}-step-${index}`}>{step}</li>
                  ))}
                </ol>
              ) : null}

              <div className="mt-3 flex items-center justify-end">
                <button
                  className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-xs font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
                  type="button"
                  onClick={() => void handleTest(channel.name)}
                  disabled={pendingChannelName === channel.name}
                >
                  {pendingChannelName === channel.name ? "Testing..." : "Test Channel"}
                </button>
              </div>

              {feedback ? (
                <p
                  className={`mt-3 rounded-lg border p-2 text-xs ${
                    feedback.type === "ok"
                      ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
                      : "border-rose-700 bg-rose-700/10 text-rose-200"
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
