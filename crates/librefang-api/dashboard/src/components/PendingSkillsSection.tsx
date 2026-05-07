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

import { useState } from "react";
import { Card } from "./ui/Card";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { CardSkeleton } from "./ui/Skeleton";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import {
  usePendingSkillCandidates,
} from "../lib/queries/skills";
import {
  useApprovePendingCandidate,
  useRejectPendingCandidate,
} from "../lib/mutations/skills";
import { formatDate } from "../lib/datetime";
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

function CandidateRow({ candidate }: { candidate: PendingCandidate }) {
  const approve = useApprovePendingCandidate();
  const reject = useRejectPendingCandidate();
  const [expanded, setExpanded] = useState(false);
  const [confirmReject, setConfirmReject] = useState(false);

  const src = sourceLabel(candidate.source);
  const busy = approve.isPending || reject.isPending;

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

  return (
    <Card className="p-4">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-semibold">
          Skill workshop pending
          {candidates.length > 0 ? (
            <Badge className="ml-2" variant="brand">
              {candidates.length}
            </Badge>
          ) : null}
        </h2>
        <p className="text-xs text-muted-foreground">
          Drafts captured from agent conversations awaiting your review (#3328).
        </p>
      </div>
      {candidates.length === 0 ? (
        <EmptyState
          title="No pending candidates"
          description="The skill workshop captures reusable workflows from successful agent conversations. Enable per-agent via [skill_workshop] in agent.toml."
        />
      ) : (
        <ul className="mt-3">
          {candidates.map((c) => (
            <CandidateRow key={c.id} candidate={c} />
          ))}
        </ul>
      )}
    </Card>
  );
}
