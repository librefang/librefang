// Skill workshop (#3328) — pending-candidate review section.
//
// Renders the workshop's after-turn capture queue and lets the operator
// approve or reject candidates. Scoped to all agents — per-agent
// filtering is exposed by the backend (`?agent=<uuid>`) and can be
// wired up here later if a per-agent SkillsPage tab gets added; the
// initial cut keeps the UI flat to keep the diff small.
//
// Data layer: `usePendingSkillCandidates` (lib/queries/skills.ts) +
// `useApprovePendingCandidate` / `useRejectPendingCandidate`
// (lib/mutations/skills.ts). No inline `fetch()` / `api.*` calls per
// the dashboard data-layer rule.

import { useMemo, useState } from "react";
import { Card } from "./ui/Card";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { CardSkeleton } from "./ui/Skeleton";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import {
  usePendingSkillCandidates,
  useSkillDetail,
} from "../lib/queries/skills";
import {
  useApprovePendingCandidate,
  useRejectPendingCandidate,
  useProposePendingToRegistry,
} from "../lib/mutations/skills";
import { formatDate } from "../lib/datetime";
import { unifiedLineDiff, hasChanges } from "../lib/unifiedDiff";
import type { PendingCandidate, PendingCaptureSource } from "../api";

function sourceLabel(source: PendingCaptureSource): {
  label: string;
  detail: string;
} {
  switch (source.kind) {
    case "explicit_instruction":
      return { label: "Explicit instruction", detail: source.trigger };
    case "user_correction":
      return { label: "User correction", detail: source.trigger };
    case "repeated_tool_pattern":
      return {
        label: "Repeated tool pattern",
        detail: `${source.tools} ×${source.repeat_count}`,
      };
  }
}

// Inline unified diff between the current skill body and the candidate's
// proposed body. Rendered for update candidates so the reviewer sees exactly
// what the agent wants to change before proposing it to the registry (#5819).
function UpdateDiffView({
  candidate,
}: {
  candidate: PendingCandidate;
}) {
  // `useSkillDetail` is gated on a non-empty name (queries/skills.ts), so for
  // a create candidate (no target) this is a no-op disabled query.
  const targetName = candidate.target_skill_id ?? "";
  const detail = useSkillDetail(targetName);
  const currentBody = detail.data?.prompt_context ?? "";

  const diff = useMemo(
    () => unifiedLineDiff(currentBody, candidate.prompt_context),
    [currentBody, candidate.prompt_context],
  );

  const versionLabel =
    candidate.current_version || candidate.proposed_version ? (
      <span className="font-mono">
        {candidate.current_version ?? "?"} → {candidate.proposed_version ?? "?"}
      </span>
    ) : null;

  return (
    <div className="mt-2 rounded border border-border/40 bg-muted/40 p-2 text-xs">
      <div className="mb-1 flex items-center gap-2 font-medium">
        <span>Proposed changes</span>
        {versionLabel}
      </div>
      {detail.isLoading ? (
        <p className="text-muted-foreground">Loading current skill body…</p>
      ) : detail.isError ? (
        <p className="text-destructive">
          Could not load current skill body to diff:{" "}
          {(detail.error as Error)?.message ?? "unknown"}
        </p>
      ) : !hasChanges(diff) ? (
        <p className="text-muted-foreground">
          No differences between the current and proposed body.
        </p>
      ) : (
        <pre className="overflow-x-auto whitespace-pre font-mono leading-snug">
          {diff.map((line, idx) => {
            const prefix =
              line.kind === "add" ? "+" : line.kind === "remove" ? "-" : " ";
            const cls =
              line.kind === "add"
                ? "bg-success/15 text-success"
                : line.kind === "remove"
                  ? "bg-destructive/15 text-destructive"
                  : "text-muted-foreground";
            return (
              <div key={idx} className={cls}>
                {prefix} {line.text}
              </div>
            );
          })}
        </pre>
      )}
    </div>
  );
}

// A candidate is an update/patch when the backend tagged it `kind: "update"`
// or it carries a target skill / version metadata (legacy drafts may omit
// `kind`, which defaults to `create` server-side).
function isUpdateCandidate(candidate: PendingCandidate): boolean {
  return (
    candidate.kind === "update" ||
    !!candidate.target_skill_id ||
    !!candidate.current_version
  );
}

function CandidateRow({ candidate }: { candidate: PendingCandidate }) {
  const approve = useApprovePendingCandidate();
  const reject = useRejectPendingCandidate();
  const propose = useProposePendingToRegistry();
  const [expanded, setExpanded] = useState(false);
  const [confirmReject, setConfirmReject] = useState(false);

  const src = sourceLabel(candidate.source);
  const isUpdate = isUpdateCandidate(candidate);
  const busy = approve.isPending || reject.isPending || propose.isPending;
  const proposedPrUrl =
    propose.isSuccess && propose.data ? propose.data.pr_url : null;

  return (
    <li className="border-b border-border/40 py-3 last:border-b-0">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-sm font-medium">
              {candidate.name}
            </span>
            <Badge variant="default" className="text-xs">
              {src.label}
            </Badge>
            {isUpdate ? (
              <Badge variant="brand" className="text-xs">
                Update
                {candidate.current_version || candidate.proposed_version
                  ? ` ${candidate.current_version ?? "?"} → ${
                      candidate.proposed_version ?? "?"
                    }`
                  : ""}
              </Badge>
            ) : null}
            <span
              className="truncate text-xs text-muted-foreground"
              title={src.detail}
            >
              {src.detail}
            </span>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {candidate.description}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            Captured {formatDate(candidate.captured_at)} · agent{" "}
            <span className="font-mono">
              {candidate.agent_id.slice(0, 8)}…
            </span>
          </p>
          {isUpdate ? <UpdateDiffView candidate={candidate} /> : null}
          {expanded ? (
            <div className="mt-2 rounded border border-border/40 bg-muted/40 p-2 text-xs">
              <div className="mb-1 font-medium">User message excerpt</div>
              <pre className="whitespace-pre-wrap break-words font-mono">
                {candidate.provenance.user_message_excerpt}
              </pre>
              {candidate.provenance.assistant_response_excerpt ? (
                <>
                  <div className="mb-1 mt-3 font-medium">
                    Assistant response excerpt
                  </div>
                  <pre className="whitespace-pre-wrap break-words font-mono">
                    {candidate.provenance.assistant_response_excerpt}
                  </pre>
                </>
              ) : null}
              <div className="mb-1 mt-3 font-medium">Body draft</div>
              <pre className="whitespace-pre-wrap break-words font-mono">
                {candidate.prompt_context}
              </pre>
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-col gap-1">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setExpanded((v) => !v)}
            disabled={busy}
          >
            {expanded ? "Hide" : "Details"}
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => approve.mutate({ id: candidate.id })}
            disabled={busy}
          >
            {approve.isPending ? "Approving…" : "Approve"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => propose.mutate({ id: candidate.id })}
            disabled={busy}
            title="Open a PR contributing this draft to the public skill registry"
          >
            {propose.isPending ? "Proposing…" : "Propose to Registry"}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setConfirmReject(true)}
            disabled={busy}
          >
            Reject
          </Button>
        </div>
      </div>
      {approve.isError ? (
        <div
          className="mt-2 rounded border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive"
          role="alert"
        >
          Approve failed: {(approve.error as Error)?.message ?? "unknown"}
        </div>
      ) : null}
      {propose.isError ? (
        <div
          className="mt-2 rounded border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive"
          role="alert"
        >
          Propose to registry failed:{" "}
          {(propose.error as Error)?.message ?? "unknown"}
        </div>
      ) : null}
      {proposedPrUrl ? (
        <div className="mt-2 rounded border border-success/30 bg-success/10 p-2 text-xs">
          Pull request opened:{" "}
          <a
            href={proposedPrUrl}
            target="_blank"
            rel="noreferrer"
            className="font-medium text-brand underline"
          >
            {proposedPrUrl}
          </a>
        </div>
      ) : null}
      <ConfirmDialog
        isOpen={confirmReject}
        onClose={() => setConfirmReject(false)}
        onConfirm={() => {
          reject.mutate(
            { id: candidate.id },
            { onSuccess: () => setConfirmReject(false) },
          );
        }}
        title="Reject candidate?"
        message={`The pending candidate '${candidate.name}' will be deleted. This cannot be undone.`}
        confirmLabel={reject.isPending ? "Rejecting…" : "Reject"}
        tone="destructive"
      />
    </li>
  );
}

export function PendingSkillsSection() {
  const query = usePendingSkillCandidates();
  const candidates = query.data ?? [];

  if (query.isLoading) {
    return <CardSkeleton />;
  }
  if (query.isError) {
    return (
      <Card className="p-4">
        <h2 className="text-base font-semibold">Skill workshop pending</h2>
        <p className="mt-2 text-sm text-destructive">
          Failed to load pending candidates:{" "}
          {(query.error as Error)?.message ?? "unknown error"}
        </p>
      </Card>
    );
  }

  // Empty queue is the steady state — most users never have a pending
  // candidate. Render nothing rather than a permanent ~150 px Card +
  // EmptyState block on the Skills page; the section materialises only
  // when there is actually something to review. Discoverability of the
  // workshop feature lives in `docs/architecture/skill-workshop.md`
  // and the `[skill_workshop]` block of `agent.toml`.
  if (candidates.length === 0) {
    return null;
  }

  return (
    <Card className="p-4">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-semibold">
          Skill workshop pending
          <Badge className="ml-2" variant="brand">
            {candidates.length}
          </Badge>
        </h2>
        <p className="text-xs text-muted-foreground">
          Drafts captured from agent conversations awaiting your review (#3328).
        </p>
      </div>
      <ul className="mt-3">
        {candidates.map((c) => (
          <CandidateRow key={c.id} candidate={c} />
        ))}
      </ul>
    </Card>
  );
}
