---
name: wiki-librarian
version: "1.0.0"
description: Wiki Librarian skills — schema management, indexing, linting resolution, and overall vault health.
author: Leszek3737
tags: [wiki, knowledge-base, librarian, schema, maintenance]
runtime: prompt_only
---

# Wiki Hand — SKILL-main.md

## 1. Obsidian Conventions

### Wikilinks
- Internal cross-references: `[[page-name]]` (no path, no extension)
- Display text override: `[[page-name|Display Text]]`
- Never use markdown `[text](path)` for internal links
- External URLs: standard markdown `[text](https://...)`
- Image embeds: `![[filename.png]]`
- Images live in `raw/assets/`

### File Naming
- Format: `kebab-case.md`
- Max 40 characters (excluding `.md`)
- ASCII only — transliterate non-ASCII characters (ü → ue, ñ → n, ś → s, etc.)
- Entities: canonical name (`john-doe.md`, not `dr-john-doe-phd.md`)
- Concepts: noun phrase (`knowledge-base.md`, not `about-knowledge-bases.md`)
- Sources: derived from title (`the-future-of-ai.md`)
- Syntheses: derived from query (`tradeoffs-x-vs-y.md`)
- No version suffixes (_v2, _revised, _updated)

### Frontmatter
- Valid YAML between `---` delimiters at the top of every page
- All fields lowercase with underscores
- Dates: ISO 8601 (`YYYY-MM-DD`)
- Lists: YAML sequences (not comma-separated strings)
- Dataview-compatible: all fields are queryable

### Dataview Compatibility
Useful queries the user can run in Obsidian:
```dataview
TABLE confidence, source_count, last_updated
FROM "pages/entities"
SORT source_count DESC
```
```dataview
LIST
FROM "pages/concepts"
WHERE confidence = "disputed"
```
```dataview
TABLE claim_count, date_ingested
FROM "pages/sources"
SORT date_ingested DESC
```

---

## 3. Index and Log Formats

### index.md Structure

```markdown
# Wiki Index

## 5. Cross-Referencing Patterns

### When to Create a Dedicated Page

| Condition | Action |
|-----------|--------|
| Entity/concept is the main subject of a source | Create page |
| Entity/concept appears substantively in 3+ sources | Create page |
| Entity is the author and contributes beyond a byline | Create page |
| Passing mention in < 3 sources, not the main subject | Plain text only, NO page, NO wikilink |
| Librarian or user explicitly requests a page | Create regardless of threshold |

"Substantively" means discussed in at least one paragraph, not just name-dropped.

### Wikilink Rules
- ONLY link to pages that exist. Never create a [[wikilink]] to a non-existent page.
- When creating a new page during ingest, you may wikilink to it from other pages you're touching in the same operation.
- First mention per section: use wikilink `[[page-name]]`
- Subsequent mentions in the same section: plain text is fine
- On creating a new page: check if other existing pages mention this entity/concept in plain text and could now be wikilinked. Flag this in the manifest for Librarian to handle.

### Backlink Maintenance
- On page creation: add the new page to relevant existing pages' "See Also" sections
- On page deletion: replace all inbound [[wikilinks]] with plain text
- On merge: redirect all inbound [[wikilinks]] from deleted page to surviving page

---

## 9. Schema Evolution Patterns

### How to Propose Changes

When Librarian identifies a recurring pattern:

1. State the observation: "I've noticed several entities are research papers. Currently we classify these as tools."
2. Propose a minimal change: "Add `entity_kind: paper` to the entity page schema."
3. Show the before/after diff
4. Wait for user confirmation
5. Optionally suggest a lint pass to retroactively update existing pages

### Backward Compatibility

- New optional fields: add with a default value. Existing pages remain valid.
- New required fields: add as optional first, run lint to find pages missing the field, then promote to required.
- New page types: add template to schema.md, create directory if needed.
- Changed field names: rename in all pages, update schema, lint to verify.

### Common Schema Additions

| Change | When | Example |
|--------|------|---------|
| New `entity_kind` | 3+ entities don't fit existing kinds | `paper`, `event`, `dataset` |
| New tag namespace | Domain-specific taxonomy emerging | `ai/`, `bio/`, `finance/` |
| New frontmatter field | Recurring metadata across pages | `relevance_score`, `review_status` |
| New page type | Distinct content pattern | `timeline`, `glossary`, `comparison` |

---

## 10. Configuration & Settings Reference

The Wiki Hand behavior is governed by settings in `HAND.toml`. The agents should adjust their operations based on these configurations:

### `vault_path`
- **What it is:** The root directory for the Obsidian-compatible wiki.
- **Agent Behavior:** All `shell_exec`, `file_read`, `file_write`, and `file_list` operations must be relative to or prefixed with this path.
- **Best Practice:** Never assume the vault is in the current working directory. Always use the parameterized `{vault_path}`.

### `file_back_mode`
- **What it is:** Determines whether new syntheses generated during user queries are saved back to the wiki.
- **Agent Behavior:**
  - `auto`: Automatically save to `pages/syntheses/`, update index, log, and commit.
  - `ask`: Ask the user for permission before saving.
  - `never`: Only provide the answer in chat. Do not persist the synthesis page.

### `search_backend`
- **What it is:** Specifies the mechanism for querying the wiki.
- **Agent Behavior:**
  - `index`: Read `index.md` and visually scan entries. Best for smaller wikis (<200 pages).
  - `qmd`: Use the `qmd search` CLI command. Required for larger wikis to avoid context limits.
- **Best Practice:** Proactively suggest switching to `qmd` when the wiki grows beyond 150-200 pages.

### `language`
- **What it is:** The configured language for the wiki content.
- **Agent Behavior:** All generated content, summaries, page titles, and synthesis responses must match this language, even if the user queries in another language or provides foreign-language source materials.

---

## 11. Common Pitfalls & Best Practices

### Tag Proliferation
- **Pitfall:** Creating dozens of highly specific, single-use tags (e.g., `#startup-founded-in-2023`).
- **Best Practice:** Stick to broad, thematic tags (e.g., `#startup`, `#technology`). If more specificity is needed, use `entity_kind` or create a synthesis page grouping them.

### Premature Page Creation (Stubs)
- **Pitfall:** Creating a dedicated entity page for something mentioned only once in passing.
- **Best Practice:** Wait until an entity has 2+ sources or significant context before promoting it to a dedicated page. Otherwise, leave it as a plain-text mention or a generic wikilink in the source summary.

### Broken YAML Frontmatter
- **Pitfall:** Generating invalid YAML (e.g., unescaped quotes in titles, incorrect list formatting for aliases).
- **Best Practice:** Always validate frontmatter syntax mentally before writing. Use arrays correctly: `aliases: ["Name 1", "Name 2"]` or list format. Quote string values if they contain colons.

### Losing Provenance
- **Pitfall:** Extracting a bold claim into an entity page without adding the inline `[[Source]]` tag.
- **Best Practice:** Every key claim or fact MUST have an inline source reference. Information without provenance decays the reliability of the entire wiki.

### Unnecessary Duplication
- **Pitfall:** Creating `Acme Corp.md` when `Acme Corporation.md` already exists, resulting in fragmented knowledge.
- **Best Practice:** Always use cross-category search or `grep` before creating new entity pages. Merge duplicate entries during routine linting.

---

## 2. Page Templates

### 2.1 Source Summary

```markdown
---
type: source
title: "{Original Title}"
author: "{Author Name}"
date_published: YYYY-MM-DD
date_ingested: YYYY-MM-DD
raw_path: "raw/{filename.ext}"
source_url: "{url or null}"
format: md
claim_count: 0
confidence: medium
tags:
  - {tag}
---

# {Title}

{2-3 sentence summary in the configured language.}

## 12. Worked Examples
