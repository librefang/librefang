---
name: wiki-linter
version: "1.0.0"
description: Wiki Linter skills — structural checks, provenance validation, contradiction detection, and vault health.
author: Leszek3737
tags: [wiki, knowledge-base, linter, validation, quality]
runtime: prompt_only
---

# Wiki Hand — SKILL-linter.md

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

## 8. Lint Checks Reference

### Check Definitions

| Check | Severity | Method |
|-------|----------|--------|
| Missing frontmatter | critical | Parse YAML — absent or malformed |
| Missing required field | critical | Compare against schema.md |
| Dead wikilink | critical | `find pages/ -name "{name}.md"` for each [[link]] |
| No provenance tag | critical | Scan Key Claims/Points/Facts bullets for missing tag |
| Index/file mismatch | critical | Compare index.md entries with actual files |
| Orphan page | warning | `grep -rl` across ALL pages/ for inbound links |
| Contradiction | warning | Compare extracted claims across pages sharing tags |
| Stale content | warning | `last_updated` vs newest source's `date_ingested` |
| Weak provenance | warning | `(inferred)` with source_count=1, or `high` confidence with source_count<2 |
| Duplicate filenames | warning | Fuzzy match on filenames (e.g., acme-corp + acme-corporation) |
| File not in index | warning | File exists but no index.md entry |
| Missing page | info | Plain-text entity/concept in 3+ source summaries |
| Large page | info | Word count > 3000 |

### Auto-Fixable vs Requires Confirmation

**Auto-fixable** (Librarian applies directly):
- Missing `last_updated` → set to today
- File missing from index.md → add entry with description from page's first paragraph
- Index entry without corresponding file → remove entry

**Requires user confirmation:**
- Contradictions (user decides which claim is correct)
- Merge recommendations (user reviews combined content)
- Delete recommendations (user accepts information loss)
- Confidence changes (user validates reasoning)
- New page creation (user approves topic importance)

---

## 12. Worked Examples
