---
name: wiki-ingestor
version: "1.0.0"
description: Wiki Ingestor skills — extraction heuristics, source/entity/concept templates, provenance, and cross-referencing.
author: Leszek3737
tags: [wiki, knowledge-base, ingestor, extraction, provenance]
runtime: prompt_only
---

# Wiki Hand — SKILL-ingestor.md

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

## 6. Extraction Heuristics

### Entity Recognition

| Entity Kind | Signals | Examples |
|-------------|---------|----------|
| `person` | Proper name, pronouns, job titles, biographical context | "John Doe, CEO of Acme" |
| `organization` | Company names, institutions, teams, brands | "Google", "MIT", "the W3C" |
| `tool` | Software, libraries, frameworks, products, protocols | "PostgreSQL", "React", "gRPC" |
| `place` | Geographic names, facilities, regions | "Silicon Valley", "CERN", "the EU" |

**Disambiguation:** When the same name could refer to different entities (e.g., "Mercury" — planet, element, car brand), use context. Create separate pages with qualifiers: `mercury-planet.md`, `mercury-element.md`. Add all variants to `aliases` in frontmatter.

### Concept Identification

Concepts are abstract ideas, patterns, theories, techniques, or methodologies — they don't refer to a specific named thing.

| Signal | Example |
|--------|---------|
| Defined or explained in the source | "Microservices architecture is a design approach where..." |
| Compared or contrasted with alternatives | "Unlike monolithic systems, microservices..." |
| Listed as a technique, methodology, or pattern | "Key patterns include: CQRS, event sourcing, and saga" |
| Forms the basis of an argument | "The efficient market hypothesis suggests..." |

### Claim Extraction

A claim is a factual assertion that can be verified, challenged, or updated. Aim for 5-15 per source.

**IS a claim:**
- "Revenue grew 40% in 2024" — verifiable quantitative data
- "The system uses a Rust backend" — architectural fact
- "The study found no significant correlation" — research finding
- "Founded in 2019" — dated fact
- "The team grew from 12 to 85 engineers" — organizational data

**NOT a claim:**
- "This is an interesting approach" — opinion without substance
- "The report continues with..." — structural description
- "See section 3 for details" — internal reference
- "It's important to consider..." — filler

### Relationship Mapping

When extracting entities, note relationships for the Relationships/Connections sections:

| Type | Example | Notation |
|------|---------|----------|
| Organizational | "Jane is CTO of Acme" | [[jane-doe]] ↔ [[acme-corp]], "CTO of" |
| Collaborative | "Developed by MIT and Google" | [[mit]] ↔ [[google]], "co-developed" |
| Competitive | "Competing with PostgreSQL" | [[product]] ↔ [[postgresql]], "competitor" |
| Dependency | "Built on React" | [[product]] ↔ [[react]], "depends on" |
| Temporal | "Founded 2019, acquired 2023" | Dates in frontmatter + key facts |
| Causal | "Migration caused 2.3x cost increase" | Noted as claim with provenance |

### Source Format Handling

| Format | Preprocessing | Notes |
|--------|--------------|-------|
| `.md` | None needed | Read directly |
| `.txt` | None needed | Read directly |
| `.html` | Strip tags or use web_fetch | Preserve headings structure if possible |
| `.pdf` | Extract text via pdftotext | May lose formatting; note if tables are garbled |
| Images in source | Note `![[image.png]]` references | If LLM vision available, describe key images |

When a source contains images that convey information (charts, diagrams, screenshots): if vision capability is available, describe the image content in the Assessment section. If not, note "Source contains visual content not processed: {description of what images appear to show}."

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

### Example A: Ingesting a Technical Article

**Source:** `raw/microservices-at-scale-2025.md` — a blog post by Jane Chen about Acme Corp's migration from monolith to microservices.

**Step 1 — Analyze:** Ingestor reads the source and identifies:
- Entities: Jane Chen (person), Acme Corp (organization), Kubernetes (tool), AWS (organization)
- Concepts: microservices architecture, service mesh, circuit breaker pattern
- Key claims: 8 factual assertions

**Step 2 — Classify thresholds:**
- Jane Chen → CREATE (author, contributes substantively)
- Acme Corp → CREATE (main subject)
- Kubernetes → MENTION-ONLY (passing mention, 1 source, below threshold)
- AWS → MENTION-ONLY (passing mention, 1 source)
- microservices architecture → CREATE (main topic)
- service mesh → CREATE (discussed substantively)
- circuit breaker pattern → CREATE (discussed with specific data)

**Step 3 — Source summary → `pages/sources/microservices-at-scale-2025.md`:**

```markdown
---
type: source
title: "Microservices at Scale: Lessons from Acme Corp"
author: "Jane Chen"
date_published: 2025-09-15
date_ingested: 2026-04-09
raw_path: "raw/microservices-at-scale-2025.md"
source_url: "https://blog.acme.com/microservices-at-scale"
format: md
claim_count: 8
confidence: medium
tags:
  - architecture
  - cloud
---

# Microservices at Scale: Lessons from Acme Corp

Jane Chen describes Acme Corp's two-year migration from a monolithic Rails application to a microservices architecture on Kubernetes. The post covers technical decisions, organizational challenges, and performance outcomes.
