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
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Skills</h1>
          <p className="text-sm text-slate-400">Installed skill modules and runtime capabilities.</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300">
            {skills.length} installed
          </span>
          <button
            className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void skillsQuery.refetch()}
            disabled={skillsQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {skillsError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{skillsError}</div>
      ) : null}

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

      <form
        className="flex flex-col gap-2 rounded-xl border border-slate-800 bg-slate-900/70 p-4 sm:flex-row sm:items-center"
        onSubmit={handleInstall}
      >
        <label className="text-sm text-slate-300" htmlFor="skill-name">
          Install skill by name
        </label>
        <input
          id="skill-name"
          type="text"
          value={installName}
          onChange={(event) => setInstallName(event.target.value)}
          placeholder="e.g. github-helper"
          className="w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none ring-sky-500/70 transition focus:border-sky-500 focus:ring"
          disabled={installMutation.isPending}
        />
        <button
          className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
          type="submit"
          disabled={installMutation.isPending || installName.trim().length === 0}
        >
          {installMutation.isPending ? "Installing..." : "Install"}
        </button>
      </form>

      {skillsQuery.isLoading && skills.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">Loading skills...</div>
      ) : null}

      {!skillsQuery.isLoading && skills.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">No skills installed.</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {skills.map((skill) => (
          <article key={skill.name} className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
            <div className="mb-3 flex items-start justify-between gap-3">
              <div>
                <h2 className="m-0 text-base font-semibold">{skill.name}</h2>
                <p className="text-xs text-slate-500">{skill.version ?? "version unknown"}</p>
              </div>
              <span
                className={`rounded-full border px-2 py-1 text-[11px] ${
                  skill.enabled
                    ? "border-emerald-700 bg-emerald-700/20 text-emerald-300"
                    : "border-slate-700 bg-slate-800/60 text-slate-300"
                }`}
              >
                {skill.enabled ? "Enabled" : "Disabled"}
              </span>
            </div>

            <p className="mb-2 text-sm text-slate-300">{skill.description || "No description."}</p>

            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
              <dt className="text-slate-400">Runtime</dt>
              <dd>{skill.runtime ?? "-"}</dd>
              <dt className="text-slate-400">Author</dt>
              <dd>{skill.author || "-"}</dd>
              <dt className="text-slate-400">Tools</dt>
              <dd>{skill.tools_count ?? 0}</dd>
            </dl>

            {skill.tags && skill.tags.length > 0 ? (
              <div className="mt-3 flex flex-wrap gap-1">
                {skill.tags.map((tag) => (
                  <span
                    key={`${skill.name}-${tag}`}
                    className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-[11px] text-slate-300"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            ) : null}

            <div className="mt-3 flex justify-end">
              <button
                className="rounded-lg border border-rose-700 bg-rose-700/20 px-3 py-2 text-xs font-medium text-rose-200 transition hover:bg-rose-700/30 disabled:cursor-not-allowed disabled:opacity-60"
                type="button"
                onClick={() => void handleUninstall(skill.name)}
                disabled={pendingUninstallName === skill.name}
              >
                {pendingUninstallName === skill.name ? "Uninstalling..." : "Uninstall"}
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
