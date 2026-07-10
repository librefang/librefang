---
name: wiki-analyst
version: "1.0.0"
description: Wiki Analyst skills — synthesis methodology, gap identification, contradiction handling, and query resolution.
author: Leszek3737
tags: [wiki, knowledge-base, analyst, synthesis, reasoning]
runtime: prompt_only
---

# Wiki Hand — SKILL-analyst.md

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

## 4. Provenance and Confidence

### Inline Provenance Syntax

Every key claim (bullet point in Key Claims, Key Points, Key Facts sections) requires a provenance tag at the end of the line:

```markdown
- The company reported $10M ARR in Q4 2025 ([[annual-report-2025]], extracted)
- This suggests a 40% year-over-year growth rate (inferred)
- However, a later filing revised this to $8.5M ([[sec-filing-q1-2026]], extracted)
- The actual growth rate remains disputed ([[annual-report-2025]], [[sec-filing-q1-2026]], disputed)
```

| Tag | Meaning | When to use |
|-----|---------|-------------|
| `([[source]], extracted)` | Directly stated in the source | Verbatim facts, statistics, dates, names |
| `([[source-a]], [[source-b]], extracted)` | Corroborated across sources | Same fact confirmed independently |
| `(inferred)` | Derived by the LLM | Connections, implications, patterns not explicitly stated |
| `([[source-a]], [[source-b]], disputed)` | Sources contradict | Present both claims, let the user judge |

### Page-Level Confidence

Set in frontmatter `confidence` field:

| Level | Rule |
|-------|------|
| `high` | All key claims are `extracted` from 2+ corroborating sources |
| `medium` | Mix of extracted and inferred, OR all extracted from a single source |
| `low` | Primarily inferred, OR based on one unverified source |
| `disputed` | Contains at least one claim tagged `disputed` |

**Propagation in syntheses:** A synthesis page's confidence = the MINIMUM confidence of its consulted pages. If any consulted page is `disputed`, the synthesis must flag this.

### Confidence Update Triggers
- New source corroborates an existing claim → consider upgrading to `high`
- New source contradicts an existing claim → downgrade to `disputed`
- Source retracted or superseded → re-assess claims dependent on it

---

## 7. Synthesis Methodology

### Combining Multiple Sources

1. **Identify overlapping claims** — same fact from different sources
2. **Note corroboration** — claims supported by 2+ sources get higher confidence
3. **Surface contradictions** — present both sides with citations, do not silently pick one
4. **Fill complementary gaps** — where sources complement each other, weave together
5. **Maintain provenance chain** — every claim in the synthesis traces back to source pages

### Handling Contradictions

```markdown

## 2. Page Templates

## 12. Worked Examples
