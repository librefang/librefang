import { useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { ApiError } from "../lib/http/errors";
import {
  IDENTITY_FIELD_KEYS,
  IDENTITY_FOREIGN_KEYS,
  identityFieldsDiffer,
  parseIdentityFrontMatter,
  readIdentityField,
  readIdentityFields,
  serializeIdentityFrontMatter,
  type IdentityFieldKey,
  type IdentityFrontMatter,
} from "../lib/identityFrontMatter";
import { useSetAgentFile } from "../lib/mutations/agentFiles";
import { useAgentFile } from "../lib/queries/agentFiles";
import { useUIStore } from "../lib/store";
import { Button } from "./ui/Button";
import { Field } from "./ui/Field";
import { Input } from "./ui/Input";

/** The workspace file this editor owns. The daemon's allowlist also admits SOUL.md, USER.md, TOOLS.md, MEMORY.md, AGENTS.md, BOOTSTRAP.md and HEARTBEAT.md. */
export const IDENTITY_FILE_NAME = "IDENTITY.md";

export interface AgentIdentityFileEditorProps {
  /** The agent whose workspace holds the file. */
  agentId: string;
  /** Defaults to `IDENTITY.md`; the prop exists so the same editor can be reused for a sibling identity file. */
  filename?: string;
  className?: string;
}

/** An empty document for an agent that has no file yet — saving writes one. */
const EMPTY_DOCUMENT: IdentityFrontMatter = {
  kind: "absent",
  bom: "",
  content: "",
  newline: "\n",
};

/**
 * The daemon refuses an identity file over 32 KiB, counted in bytes of the
 * JSON string (`MAX_FILE_SIZE` in
 * `crates/librefang-api/src/routes/agents/files.rs:422`).
 *
 * Measuring here instead of letting the 413 come back is deliberate: the
 * daemon answers that status with a Fluent template whose `$max` argument the
 * handler never passes, so the operator would be shown the literal
 * `File too large (max { $max })` — a message that names no limit, no size and
 * no file, and which nothing in this UI can act on. Bytes, not characters: a
 * file of CJK or emoji reaches 32 KiB well before it reaches 32 768 characters.
 */
const MAX_IDENTITY_FILE_BYTES = 32_768;

type Draft = Record<IdentityFieldKey, string>;

/** English defaults for the three field labels, also the i18n fallback. */
const IDENTITY_FIELD_DEFAULTS: Record<IdentityFieldKey, string> = {
  archetype: "Archetype",
  vibe: "Vibe",
  greeting_style: "Greeting style",
};

function emptyDraft(): Draft {
  return { archetype: "", vibe: "", greeting_style: "" };
}

function draftFrom(document: IdentityFrontMatter): Draft {
  return document.kind === "malformed" ? emptyDraft() : readIdentityFields(document);
}

/**
 * Structured editor for the front-matter of a workspace identity file.
 *
 * `IDENTITY.md` is injected verbatim into the agent's system prompt under
 * `## Identity` (capped at 500 chars) and nothing on that path parses it, so
 * `archetype`, `vibe` and `greeting_style` are keys the model reads as prose
 * rather than a schema anything enforces. They were previously reachable only
 * by editing markdown by hand; this gives them fields without taking the file
 * away from the operator.
 *
 * Those same three names also exist on the agent record
 * (`librefang_types::agent::AgentIdentity`, reachable through
 * `PATCH /api/agents/{id}/identity`), and the two stores never reconcile — only
 * the file reaches the prompt. This editor writes the file and only the file,
 * so a value set on the record will not appear here and will not reach the
 * model either.
 *
 * Three keys are editable and nothing else is. The file's body, its other
 * keys, their order and its line endings survive a save byte for byte
 * (`lib/identityFrontMatter.ts`), because a rewrite-from-scratch would silently
 * destroy whatever the operator wrote around this block.
 *
 * Saving is explicit and writes the file, not the manifest — it is not folded
 * into the manifest form's save, which would make one button write two stores.
 */
export function AgentIdentityFileEditor({
  agentId,
  filename = IDENTITY_FILE_NAME,
  className = "",
}: AgentIdentityFileEditorProps) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const fieldIdPrefix = useId();

  const fileQuery = useAgentFile(agentId, filename);
  const saveFile = useSetAgentFile(agentId, filename);

  const content = fileQuery.data?.content;
  const parsed = useMemo(
    () => (typeof content === "string" ? parseIdentityFrontMatter(content) : null),
    [content],
  );

  // The draft is state so typing does not touch the query cache; it re-seeds
  // whenever the underlying bytes change — after a save, or when the panel
  // switches to another agent — so the form never shows a value the file no
  // longer holds.
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  useEffect(() => {
    if (parsed) setDraft(draftFrom(parsed));
  }, [parsed]);

  const missing = fileQuery.error instanceof ApiError && fileQuery.error.status === 404;
  const document = parsed ?? EMPTY_DOCUMENT;
  const readOnly = document.kind === "malformed";
  const dirty = !readOnly && identityFieldsDiffer(document, draft);

  const serialized = useMemo(
    () => (readOnly ? null : serializeIdentityFrontMatter(document, draft)),
    [document, draft, readOnly],
  );

  const serializedBytes = useMemo(
    () => (serialized === null ? 0 : new TextEncoder().encode(serialized).length),
    [serialized],
  );
  const tooLarge = serializedBytes > MAX_IDENTITY_FILE_BYTES;

  const handleSave = () => {
    // Truthiness rather than a null check: an empty string would be a valid
    // PUT that truncates the file, and no reachable state produces one.
    if (!serialized || tooLarge) return;
    saveFile.mutate(serialized, {
      onSuccess: () => {
        addToast(
          t("agents.identity_file.saved", {
            defaultValue: "{{name}} saved",
            name: filename,
          }),
          "success",
        );
      },
      onError: (error) => {
        addToast(
          error?.message ||
            t("agents.identity_file.save_failed", {
              defaultValue: "Could not save {{name}}",
              name: filename,
            }),
          "error",
        );
      },
    });
  };

  const title = t("agents.identity_file.title", {
    defaultValue: "Personality ({{name}})",
    name: filename,
  });

  // Exactly one notice, chosen by what the file actually is — "not there yet",
  // "there but empty", or "there with content and no block". An empty existing
  // file is a different situation from a missing one: saving appends nothing to
  // it either way, but only one of the two is a file the operator may not have
  // expected to exist.
  const notice = missing
    ? t("agents.identity_file.missing", {
        defaultValue: "This agent has no {{name}} yet. Saving creates it.",
        name: filename,
      })
    : document.kind !== "absent"
      ? null
      : document.content.length > 0
        ? t("agents.identity_file.no_front_matter", {
            defaultValue:
              "This file has no front-matter block. Saving adds one above the existing content; nothing below it is rewritten.",
          })
        : t("agents.identity_file.empty_file", {
            defaultValue:
              "This file is empty. Saving writes a front-matter block into it.",
          });

  if (fileQuery.isLoading) {
    return (
      <section className={className} aria-label={title}>
        <p className="text-xs text-text-dim" role="status">
          {t("agents.identity_file.loading", {
            defaultValue: "Reading {{name}}…",
            name: filename,
          })}
        </p>
      </section>
    );
  }

  if (fileQuery.isError && !missing) {
    return (
      <section className={className} aria-label={title}>
        <p className="text-xs text-error" role="alert">
          {t("agents.identity_file.load_failed", {
            defaultValue: "Could not read {{name}}",
            name: filename,
          })}
        </p>
        <p className="mt-1 text-[10px] text-text-dim">{fileQuery.error.message}</p>
      </section>
    );
  }

  return (
    <section className={className} aria-label={title}>
      <div className="flex flex-col gap-0.5">
        <h3 className="text-xs font-bold uppercase tracking-widest text-text-dim">
          {title}
        </h3>
        <p className="text-[10px] text-text-dim/70">
          {t("agents.identity_file.hint", {
            defaultValue:
              "This agent's personality. Injected into its prompt under “## Identity”, cut off at 500 characters. The rest of the file is written back untouched.",
          })}
        </p>
      </div>

      {notice && (
        <p className="mt-3 rounded-lg border border-border-subtle bg-main/40 px-3 py-2 text-[10px] text-text-dim">
          {notice}
        </p>
      )}

      {readOnly && (
        <div className="mt-3 rounded-lg border border-border-subtle bg-main/40 px-3 py-2" role="alert">
          <p className="text-[10px] font-bold uppercase tracking-widest text-warning">
            {t("agents.identity_file.malformed_title", {
              defaultValue: "Front-matter block not closed",
            })}
          </p>
          <p className="mt-1 text-[10px] text-text-dim">
            {t("agents.identity_file.malformed_hint", {
              defaultValue:
                "The file opens with “---” and never closes it, so the dashboard cannot tell front-matter from a horizontal rule. These fields stay read-only rather than risk rewriting the file; edit the file as text to close the block.",
            })}
          </p>
        </div>
      )}

      {!readOnly && (
        <div className="mt-4 flex flex-col gap-3">
          {IDENTITY_FIELD_KEYS.map((key) => {
            const id = `${fieldIdPrefix}-${key}`;
            return (
              <Field
                key={key}
                htmlFor={id}
                label={t(`agents.identity_file.field_${key}`, {
                  defaultValue: IDENTITY_FIELD_DEFAULTS[key],
                })}
              >
                <Input
                  id={id}
                  value={draft[key]}
                  disabled={saveFile.isPending}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, [key]: event.target.value }))
                  }
                />
              </Field>
            );
          })}
        </div>
      )}

      <ForeignKeysNote document={document} />

      <div className="mt-4 flex flex-wrap items-center gap-3">
        {!readOnly && (
          <Button
            type="button"
            variant="primary"
            size="sm"
            isLoading={saveFile.isPending}
            disabled={!dirty || tooLarge}
            onClick={handleSave}
          >
            {t("agents.identity_file.save", {
              defaultValue: "Save {{name}}",
              name: filename,
            })}
          </Button>
        )}
        {tooLarge ? (
          <span className="text-[10px] text-error" role="alert">
            {t("agents.identity_file.too_large", {
              defaultValue:
                "This file would be {{size}} bytes; the daemon accepts at most {{limit}}. Shorten a field above, or the content below the front-matter.",
              size: serializedBytes,
              limit: MAX_IDENTITY_FILE_BYTES,
            })}
          </span>
        ) : (
          <span className="text-[10px] text-text-dim/70">
            {readOnly
              ? t("agents.identity_file.read_only", {
                  defaultValue: "Read-only — the block reads as malformed.",
                })
              : dirty
                ? t("agents.identity_file.unsaved", {
                    defaultValue: "Unsaved changes. Saving writes this file only, not the manifest.",
                  })
                : t("agents.identity_file.unchanged", {
                    defaultValue: "No changes since the last read.",
                  })}
          </span>
        )}
      </div>

      {document.kind === "ok" && document.body.length > 0 && (
        <div className="mt-4">
          <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
            {t("agents.identity_file.body_title", {
              defaultValue: "Content below the front-matter",
            })}
          </p>
          <pre className="mt-1 max-h-40 overflow-auto rounded-lg border border-border-subtle bg-main/40 px-3 py-2 text-[10px] whitespace-pre-wrap text-text-dim">
            {document.body}
          </pre>
          <p className="mt-1 text-[10px] text-text-dim/70">
            {t("agents.identity_file.body_hint", {
              defaultValue: "Not editable here, and written back exactly as it is.",
            })}
          </p>
        </div>
      )}
    </section>
  );
}

/**
 * `emoji`, `avatar_url` and `color` sit in the same block but belong to the
 * Appearance section, which stores them on the agent record rather than in this
 * file. A file generated today does not carry them at all; a file written
 * before the split does, and its value is shown even when it is empty.
 *
 * A key is listed only when the block actually has a line for it. Three rows
 * reading "empty" about keys that are not in the file describe a problem the
 * operator does not have, and the operator cannot act on any of them from here.
 * What is in the file is shown rather than omitted, so they can see that the
 * editor knows about it and is deliberately leaving it alone; a second editable
 * control for a value that lives in another store is exactly the confusion this
 * panel exists to avoid.
 */
function ForeignKeysNote({ document }: { document: IdentityFrontMatter }) {
  const { t } = useTranslation();

  // `readIdentityField` answers `null` for a key with no line — an empty value
  // reads as `""`. A malformed block answers `null` for every key, which is
  // consistent with the read-only state the rest of the panel takes there: no
  // value in that block can be read at all.
  const present = IDENTITY_FOREIGN_KEYS.flatMap((key) => {
    const value = readIdentityField(document, key);
    return value === null ? [] : [{ key, value }];
  });

  if (present.length === 0) return null;

  return (
    <div className="mt-4">
      <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
        {t("agents.identity_file.foreign_title", {
          defaultValue: "Appearance",
        })}
      </p>
      <ul className="mt-1 flex flex-col gap-0.5">
        {present.map(({ key, value }) => (
          <li key={key} className="text-[10px] text-text-dim">
            <span className="font-mono text-text-dim/80">{key}</span>
            {": "}
            {value === "" ? (
              <span className="text-text-dim/50">
                {t("agents.identity_file.foreign_empty", { defaultValue: "empty" })}
              </span>
            ) : (
              <span className="font-mono">{value}</span>
            )}
          </li>
        ))}
      </ul>
      <p className="mt-1 text-[10px] text-text-dim/70">
        {t("agents.identity_file.foreign_hint", {
          defaultValue:
            "Set in Appearance, which stores them with the agent. Left untouched here.",
        })}
      </p>
    </div>
  );
}
