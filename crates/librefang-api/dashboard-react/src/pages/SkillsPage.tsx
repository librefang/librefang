import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FormEvent, useMemo, useState } from "react";
import { installSkill, listSkills, uninstallSkill, type ApiActionResponse } from "../api";

const REFRESH_MS = 30000;

interface ActionFeedback {
  type: "ok" | "error";
  text: string;
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  return JSON.stringify(action);
}

export function SkillsPage() {
  const queryClient = useQueryClient();
  const [installName, setInstallName] = useState("");
  const [feedback, setFeedback] = useState<ActionFeedback | null>(null);
  const [pendingUninstallName, setPendingUninstallName] = useState<string | null>(null);

  const skillsQuery = useQuery({
    queryKey: ["skills", "list"],
    queryFn: listSkills,
    refetchInterval: REFRESH_MS
  });

  const installMutation = useMutation({
    mutationFn: installSkill
  });
  const uninstallMutation = useMutation({
    mutationFn: uninstallSkill
  });

  const skills = useMemo(
    () => [...(skillsQuery.data ?? [])].sort((a, b) => a.name.localeCompare(b.name)),
    [skillsQuery.data]
  );
  const skillsError = skillsQuery.error instanceof Error ? skillsQuery.error.message : "";

  async function refreshSkills() {
    await queryClient.invalidateQueries({ queryKey: ["skills", "list"] });
    await skillsQuery.refetch();
  }

  async function handleInstall(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = installName.trim();
    if (!name || installMutation.isPending) return;

    try {
      const result = await installMutation.mutateAsync(name);
      setFeedback({ type: "ok", text: actionText(result) });
      setInstallName("");
      await refreshSkills();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Skill install failed."
      });
    }
  }

  async function handleUninstall(name: string) {
    if (uninstallMutation.isPending) return;
    if (!window.confirm(`Uninstall skill "${name}"?`)) return;

    setPendingUninstallName(name);
    try {
      const result = await uninstallMutation.mutateAsync(name);
      setFeedback({ type: "ok", text: actionText(result) });
      await refreshSkills();
    } catch (error) {
      setFeedback({
        type: "error",
        text: error instanceof Error ? error.message : "Skill uninstall failed."
      });
    } finally {
      setPendingUninstallName(null);
    }
  }

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <h1 className="m-0 text-3xl font-extrabold tracking-tight">Skills</h1>
          <p className="mt-1 text-sm text-text-dim font-medium">Installed skill modules and runtime capabilities.</p>
        </div>
        <div className="flex items-center gap-3">
          <span className="rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim">
            {skills.length} installed
          </span>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
            type="button"
            onClick={() => void skillsQuery.refetch()}
            disabled={skillsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {skillsError ? (
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-error font-bold">{skillsError}</div>
      ) : null}

      {feedback ? (
        <div
          className={`rounded-xl border p-4 text-sm font-bold shadow-sm ${
            feedback.type === "ok"
              ? "border-success/20 bg-success/5 text-success"
              : "border-error/20 bg-error/5 text-error"
          }`}
        >
          {feedback.text}
        </div>
      ) : null}

      <form
        className="flex flex-col gap-4 rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm ring-1 ring-black/5 dark:ring-white/5 sm:flex-row sm:items-end"
        onSubmit={handleInstall}
      >
        <div className="flex flex-1 flex-col gap-1.5">
          <label className="text-[10px] font-black uppercase tracking-widest text-text-dim px-1" htmlFor="skill-name">
            Install skill by name
          </label>
          <input
            id="skill-name"
            type="text"
            value={installName}
            onChange={(event) => setInstallName(event.target.value)}
            placeholder="e.g. github-helper"
            className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm transition-all focus:border-brand focus:ring-2 focus:ring-brand/20 outline-none disabled:opacity-50"
            disabled={installMutation.isPending}
          />
        </div>
        <button
          className="rounded-xl bg-brand px-6 py-2 text-sm font-bold text-white shadow-lg shadow-brand/20 hover:opacity-90 transition-all disabled:opacity-50 disabled:shadow-none h-[38px]"
          type="submit"
          disabled={installMutation.isPending || installName.trim().length === 0}
        >
          {installMutation.isPending ? "Installing..." : "Install Skill"}
        </button>
      </form>

      {skillsQuery.isLoading && skills.length === 0 ? (
        <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-sm text-text-dim font-medium italic shadow-sm">
          Loading skills...
        </div>
      ) : null}

      {!skillsQuery.isLoading && skills.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border-subtle bg-surface p-8 text-center text-sm text-text-dim font-medium shadow-sm">
          No skills installed.
        </div>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {skills.map((skill) => (
          <article key={skill.name} className="flex flex-col rounded-2xl border border-border-subtle bg-surface p-5 shadow-sm ring-1 ring-black/5 dark:ring-white/5 transition-all hover:border-brand/30">
            <div className="mb-4 flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h2 className="m-0 truncate text-base font-bold tracking-tight">{skill.name}</h2>
                <p className="text-[10px] font-bold text-text-dim uppercase tracking-widest">{skill.version ?? "version unknown"}</p>
              </div>
              <span
                className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${
                  skill.enabled
                    ? "border-success/20 bg-success/10 text-success"
                    : "border-border-subtle bg-surface-hover text-text-dim"
                }`}
              >
                {skill.enabled ? "Enabled" : "Disabled"}
              </span>
            </div>

            <p className="mb-4 flex-1 text-sm font-medium text-text-dim/80 line-clamp-2">{skill.description || "No description."}</p>

            <div className="mb-4 space-y-2">
              <div className="flex justify-between border-b border-border-subtle pb-1">
                <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Runtime</span>
                <span className="text-[11px] font-bold">{skill.runtime ?? "-"}</span>
              </div>
              <div className="flex justify-between border-b border-border-subtle pb-1">
                <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Author</span>
                <span className="text-[11px] font-bold truncate max-w-[120px]">{skill.author || "-"}</span>
              </div>
              <div className="flex justify-between border-b border-border-subtle pb-1">
                <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim/60">Tools</span>
                <span className="text-[11px] font-bold">{skill.tools_count ?? 0}</span>
              </div>
            </div>

            {skill.tags && skill.tags.length > 0 ? (
              <div className="mb-6 flex flex-wrap gap-1.5">
                {skill.tags.map((tag) => (
                  <span
                    key={`${skill.name}-${tag}`}
                    className="rounded-lg border border-border-subtle bg-main/40 px-2 py-0.5 text-[10px] font-bold text-text-dim"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            ) : null}

            <div className="mt-auto pt-4 border-t border-border-subtle/50 flex justify-end">
              <button
                className="rounded-lg border border-error/20 bg-error/10 px-3 py-1.5 text-[10px] font-bold text-error hover:bg-error/20 transition-all shadow-sm disabled:opacity-50"
                type="button"
                onClick={() => void handleUninstall(skill.name)}
                disabled={pendingUninstallName === skill.name}
              >
                {pendingUninstallName === skill.name ? "Uninstalling..." : "Uninstall Skill"}
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
