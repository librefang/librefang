---
name: collector-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI intelligence collection — OSINT methodology, entity extraction, knowledge graphs, change detection, and sentiment analysis"
runtime: prompt_only
---

# Intelligence Collection Expert Knowledge

## OSINT Methodology

### Collection Cycle
1. **Planning**: Define target, scope, and collection requirements
2. **Collection**: Gather raw data from open sources
3. **Processing**: Extract entities, relationships, and data points
4. **Analysis**: Synthesize findings, identify patterns, detect changes
5. **Dissemination**: Generate reports, alerts, and updates
6. **Feedback**: Refine queries based on what worked and what didn't

### Source Categories (by reliability)
| Tier | Source Type | Reliability | Examples |
|------|-----------|-------------|---------|
| 1 | Official/Primary | Very High | Company filings, government data, press releases |
| 2 | Institutional | High | News agencies (Reuters, AP), research institutions |
| 3 | Professional | Medium-High | Industry publications, analyst reports, expert blogs |
| 4 | Community | Medium | Forums, social media, review sites |
| 5 | Anonymous/Unverified | Low | Anonymous posts, rumors, unattributed claims |

### Search Query Construction by Focus Area

**Market Intelligence**:
```
"[target] market share"
"[target] industry report [year]"
"[target] TAM SAM SOM"
"[target] growth rate"
"[target] market analysis"
"[target industry] trends [year]"
```

**Business Intelligence**:
```
"[company] revenue" OR "[company] earnings"
"[company] CEO" OR "[company] leadership team"
"[company] strategy" OR "[company] roadmap"
"[company] partnerships" OR "[company] acquisition"
"[company] annual report" OR "[company] 10-K"
site:sec.gov "[company]"
```

**Competitor Analysis**:
```
"[company] vs [competitor]"
"[company] alternative"
"[company] review" OR "[company] comparison"
"[company] pricing" site:g2.com OR site:capterra.com
"[company] customer reviews" site:trustpilot.com
"switch from [company] to"
```

**Person Tracking**:
```
"[person name]" "[company]"
"[person name]" interview OR podcast OR keynote
"[person name]" site:linkedin.com
"[person name]" publication OR paper
"[person name]" conference OR summit
```

**Technology Monitoring**:
```
"[technology] release" OR "[technology] update"
"[technology] benchmark [year]"
"[technology] adoption" OR "[technology] usage statistics"
"[technology] vs [alternative]"
"[technology]" site:github.com
"[technology] roadmap" OR "[technology] changelog"
```

---

## Entity Extraction Patterns

### Named Entity Types
1. **Person**: Name, title, organization, role
2. **Organization**: Company name, type, industry, location, size
3. **Product**: Product name, company, category, version
4. **Event**: Type, date, participants, location, significance
5. **Financial**: Amount, currency, type (funding, revenue, valuation)
6. **Technology**: Name, version, category, vendor
7. **Location**: City, state, country, region
8. **Date/Time**: Specific dates, time ranges, deadlines

### Extraction Heuristics
- **Person detection**: Title + Name pattern ("CEO John Smith"), bylines, quoted speakers
- **Organization detection**: Legal suffixes (Inc, LLC), "at [Company]", domain names
- **Financial detection**: Currency symbols, "raised $X", "valued at", "revenue of"
- **Event detection**: Date + verb ("launched on", "announced at", "acquired")
- **Technology detection**: CamelCase names, version numbers, "built with", "powered by"

---

## Knowledge Graph Best Practices

### Entity Schema
```json
{
  "entity_id": "unique_id",
  "name": "Entity Name",
  "type": "person|company|product|event|technology",
  "attributes": {
    "key": "value"
  },
  "sources": ["url1", "url2"],
  "first_seen": "timestamp",
  "last_seen": "timestamp",
  "confidence": "high|medium|low"
}
```

### Relation Schema
```json
{
  "source_entity": "entity_id_1",
  "relation": "works_at|founded|competes_with|...",
  "target_entity": "entity_id_2",
  "attributes": {
    "since": "date",
    "context": "description"
  },
  "source": "url",
  "confidence": "high|medium|low"
}
```

### Common Relations
| Relation | Between | Example |
|----------|---------|---------|
| works_at | Person → Company | "Jane Smith works at Acme" |
| founded | Person → Company | "John Doe founded StartupX" |
| invested_in | Company → Company | "VC Fund invested in StartupX" |
| competes_with | Company → Company | "Acme competes with BetaCo" |
| partnered_with | Company → Company | "Acme partnered with CloudY" |
| launched | Company → Product | "Acme launched ProductZ" |
| acquired | Company → Company | "BigCorp acquired StartupX" |
| uses | Company → Technology | "Acme uses Kubernetes" |
| mentioned_in | Entity → Source | "Acme mentioned in TechCrunch" |

---

## Change Detection Methodology

### Change Classification

Every difference between the current snapshot and the previous one falls into exactly one category:

| Category | Definition | Examples |
|----------|-----------|---------|
| **Structural** | Entity appeared/disappeared, relationship added/removed | New competitor enters market, person left company, product deprecated, new partnership formed |
| **Content** | Attribute value changed on an existing entity | CEO changed, funding amount updated, version number bumped, pricing modified |
| **Metadata** | Supporting data changed but core fact is the same | New source confirms existing fact, confidence upgraded, last_seen timestamp refreshed |

### Cross-Source Deduplication

Before scoring, deduplicate overlapping data points:
1. **Normalize** entity names: strip legal suffixes (Inc, LLC, Corp), lowercase, expand common abbreviations
2. **Merge** when 2+ sources report the same fact about the same entity — keep highest confidence, list all source URLs
3. **Flag conflicts** when sources disagree on a fact (e.g., different funding amounts) — record both, mark as "conflicting — requires resolution"

### Significance Scoring Algorithm

Compute a numeric score (0-100) for each change:

```
Base score (by category):
  Structural change  = 60
  Content change     = 40
  Metadata change    =  5

Source reliability modifier (best source tier for this data point):
  Tier 1 (official/primary)   = +20
  Tier 2 (institutional)      = +10
  Tier 3 (professional)       = +5
  Tier 4-5 (community/anon)   = +0

Source freshness modifier (publication age):
  Within 24 hours   = +10
  Within 7 days     = +5
  Within 30 days    = +0
  Older than 30 days = -10

Corroboration modifier:
  Confirmed by 2+ independent sources = +10
  Single source only                  = +0
  Contradicted by another source      = -15

Focus area relevance:
  Directly matches configured focus_area = +10
  Tangentially related                   = +0

Final score = clamp(base + reliability + freshness + corroboration + relevance, 0, 100)
```

### Alert Tier Mapping

Map the computed significance score to an action tier using `change_significance_threshold` (configurable, default 60):

```
Score >= 80          → CRITICAL (immediate alert via event_publish)
  Examples: leadership change (CEO/CTO/CFO), acquisition or merger,
            major funding round (>$10M), product discontinuation,
            regulatory action, data breach

Score >= threshold   → IMPORTANT (include in next report)
  Examples: new product launch, new partnership, hiring surge (>5 roles),
            pricing change, significant competitor move, major customer win/loss

Score < threshold    → MINOR (note in report)
  Examples: blog post, minor update or patch, conference appearance,
            individual job posting, social media activity within normal range
```

### Source Reliability Filtering

Apply the configured `source_reliability_threshold` (default: tier_3) to filter low-quality data:
- **Discard** data points where ALL supporting sources fall below the threshold tier
- **Exception**: if a below-threshold source is the ONLY source for a structural change, keep it but downgrade confidence to "low" and flag for corroboration in the next cycle

---

## Sentiment Analysis Heuristics

When `track_sentiment` is enabled, classify each source's tone:

### Classification Rules
- **Positive indicators**: "growth", "innovation", "breakthrough", "success", "award", "expansion", "praise", "recommend"
- **Negative indicators**: "lawsuit", "layoffs", "decline", "controversy", "failure", "breach", "criticism", "warning"
- **Neutral indicators**: factual reporting without strong adjectives, data-only articles, announcements

### Sentiment Scoring
```
Strong positive: +2 (e.g., "Company wins major award")
Mild positive:   +1 (e.g., "Steady growth continues")
Neutral:          0 (e.g., "Company releases Q3 report")
Mild negative:   -1 (e.g., "Faces increased competition")
Strong negative: -2 (e.g., "Major data breach disclosed")
```

Track rolling average over last 5 collection cycles to detect trends.

---

## Report Templates

### Intelligence Brief (Markdown)
```markdown
# Intelligence Report: [Target]
**Date**: YYYY-MM-DD HH:MM UTC
**Collection Cycle**: #N
**Sources Processed**: X
**New Data Points**: Y

## Priority Changes
1. [CRITICAL] [Description + source]
2. [IMPORTANT] [Description + source]

## Executive Summary
[2-3 paragraph synthesis of new intelligence]

## Detailed Findings

### [Category 1]
- Finding with [source](url)
- Data point with confidence: high/medium/low

### [Category 2]
- ...

## Entity Updates
| Entity | Change | Previous | Current | Source |
|--------|--------|----------|---------|--------|

## Sentiment Trend
| Period | Score | Direction | Notable |
|--------|-------|-----------|---------|

## Collection Metadata
- Queries executed: N
- Sources fetched: N
- New entities: N
- Updated entities: N
- Next scheduled collection: [datetime]
```

---

## Source Evaluation Checklist

Before including data in the knowledge graph, evaluate:

1. **Recency**: Published within relevant timeframe? Stale data can mislead.
2. **Primary vs Secondary**: Is this the original source, or citing someone else?
3. **Corroboration**: Do other independent sources confirm this?
4. **Bias check**: Does the source have a financial or political interest in this claim?
5. **Specificity**: Does it provide concrete data, or vague assertions?
6. **Track record**: Has this source been reliable in the past?

If a claim fails 3+ checks, downgrade its confidence to "low".

---

## Worked Examples

### Example 1: Competitor Monitoring Campaign

**Scenario**: A B2B SaaS company wants continuous intelligence on three direct competitors: AlphaCloud, BetaStack, and GammaSuite.

**Step 1 — Define targets and collection requirements**

Configure the hand with:
```
target_subject: "AlphaCloud, BetaStack, GammaSuite"
focus_area: competitor
collection_depth: deep
update_frequency: daily
alert_on_changes: true
track_sentiment: true
max_sources_per_cycle: 50
```

Build the initial query set:
```
"AlphaCloud" pricing OR plans OR tiers
"AlphaCloud" product launch OR release OR update
"AlphaCloud" review site:g2.com OR site:capterra.com
"AlphaCloud" customer case study
"AlphaCloud" hiring site:linkedin.com OR site:greenhouse.io
"switch from AlphaCloud to"
(repeat for BetaStack and GammaSuite)
```

**Step 2 — Run first collection cycle**

Execute queries, fetch top results, extract entities:
```json
[
  {"type": "product", "name": "AlphaCloud v4.2", "company": "AlphaCloud", "launch_date": "2025-11-15", "source": "alphacloud.com/blog"},
  {"type": "person", "name": "Sarah Chen", "role": "New VP Engineering", "company": "BetaStack", "source": "linkedin.com/in/sarachen"},
  {"type": "event", "name": "GammaSuite Series C", "amount": "$85M", "date": "2025-11-10", "source": "techcrunch.com/2025/11/10/gammasuite-series-c"}
]
```

**Step 3 — Build knowledge graph entries**

```
knowledge_add_entity  type=company  name="AlphaCloud"   industry="SaaS"  funding_stage="Series B"
knowledge_add_entity  type=product  name="AlphaCloud v4.2"  category="cloud platform"
knowledge_add_entity  type=person   name="Sarah Chen"    role="VP Engineering"  company="BetaStack"
knowledge_add_relation source="AlphaCloud" relation="launched" target="AlphaCloud v4.2"
knowledge_add_relation source="Sarah Chen" relation="works_at" target="BetaStack"
```

**Step 4 — Process findings into change detection**

| Change | Type | Significance | Action |
|--------|------|-------------|--------|
| AlphaCloud released v4.2 with AI features | Product launch | IMPORTANT | Include in report, compare against own roadmap |
| BetaStack hired VP Engineering from FAANG | Leadership change | IMPORTANT | Track subsequent hiring patterns |
| GammaSuite raised $85M Series C | Major funding | CRITICAL | Immediate alert, expect aggressive expansion |

**Step 5 — Generate intelligence brief**

```markdown
# Competitor Intelligence Brief
**Date**: 2025-11-16 | **Cycle**: 1 | **Sources**: 47

## Priority Changes
1. [CRITICAL] GammaSuite closed $85M Series C led by Sequoia (TechCrunch, confirmed via Crunchbase)
2. [IMPORTANT] AlphaCloud shipped v4.2 with AI-assisted workflow builder
3. [IMPORTANT] BetaStack hired Sarah Chen (ex-Google) as VP Engineering

## Executive Summary
GammaSuite's large funding round signals intent to accelerate growth — expect increased
marketing spend and possible M&A activity in the next 6 months. AlphaCloud's v4.2
introduces direct feature overlap with our AI pipeline. BetaStack's engineering
leadership hire suggests a product quality push.

## Recommended Actions
- Review AlphaCloud v4.2 feature parity against our roadmap
- Monitor GammaSuite job postings for expansion signals
- Track BetaStack engineering team growth over next 3 cycles
```

---

### Example 2: Technology Landscape Mapping

**Scenario**: Map the emerging real-time AI inference landscape — track frameworks, adoption signals, key players, and performance benchmarks.

**Step 1 — Define scope and seed entities**

```
target_subject: "real-time AI inference (vLLM, TensorRT-LLM, Triton, Ollama, llama.cpp)"
focus_area: technology
collection_depth: exhaustive
update_frequency: weekly
```

Initial seed queries:
```
"real-time AI inference" benchmark 2025
"vLLM" vs "TensorRT-LLM" performance
"llama.cpp" release changelog
"AI inference" startup funding 2025
"edge AI inference" adoption enterprise
"AI inference" tokens per second benchmark
site:github.com "vLLM" stars OR contributors
site:arxiv.org "inference optimization" 2025
```

**Step 2 — Build entity graph from first sweep**

Entities collected:
```json
[
  {"type": "technology", "name": "vLLM", "version": "0.6.3", "vendor": "UC Berkeley / community", "category": "inference engine"},
  {"type": "technology", "name": "TensorRT-LLM", "version": "0.15", "vendor": "NVIDIA", "category": "inference engine"},
  {"type": "company", "name": "Groq", "industry": "AI hardware", "product": "LPU Inference Engine"},
  {"type": "number", "metric": "tokens_per_second", "value": 523, "context": "Groq Llama 3 70B", "date": "2025-10"},
  {"type": "number", "metric": "github_stars", "value": 32400, "context": "vLLM", "date": "2025-11"}
]
```

Relationships:
```
vLLM        --competes_with-->  TensorRT-LLM
vLLM        --competes_with-->  Ollama
Groq        --launched-->       "LPU Inference Engine"
NVIDIA      --launched-->       TensorRT-LLM
llama.cpp   --uses-->           GGUF format
```

**Step 3 — Track adoption signals across cycles**

| Signal Type | What to Watch | Detection Method |
|-------------|--------------|-----------------|
| GitHub velocity | Stars, forks, contributor count week-over-week | Snapshot comparison |
| Enterprise adoption | Case studies, "we migrated to X" blog posts | Keyword search |
| Benchmark results | Tokens/sec, latency, cost-per-token comparisons | Structured extraction |
| Job postings | "Experience with vLLM" in job descriptions | Job board queries |
| Conference talks | Accepted papers, keynote mentions | Conference program search |

**Step 4 — Detect trends over 4 weekly cycles**

```
Cycle 1: vLLM 31,800 stars | TensorRT-LLM 9,200 stars | Ollama 98,000 stars
Cycle 2: vLLM 32,400 stars | TensorRT-LLM 9,500 stars | Ollama 101,000 stars
Cycle 3: vLLM 33,500 stars | TensorRT-LLM 9,600 stars | Ollama 103,500 stars
Cycle 4: vLLM 35,200 stars | TensorRT-LLM 9,700 stars | Ollama 105,000 stars

Trend: vLLM accelerating (+1,700/wk avg → +1,700 last week)
       Ollama decelerating (+3,000/wk → +1,500/wk)
       TensorRT-LLM flat (~200/wk)
```

**Step 5 — Produce technology landscape report**

Include a positioning summary:

| Framework | Strengths | Weaknesses | Momentum | Best For |
|-----------|-----------|------------|----------|----------|
| vLLM | High throughput, PagedAttention | GPU-only, complex setup | Accelerating | Production serving at scale |
| TensorRT-LLM | NVIDIA optimization, low latency | Vendor lock-in, NVIDIA GPUs only | Flat | NVIDIA-stack deployments |
| Ollama | Simple UX, local-first | Lower throughput, less tunable | Decelerating | Developer experimentation |
| llama.cpp | CPU support, portable | Manual optimization needed | Steady | Edge/embedded inference |
| Groq LPU | Extreme speed, low latency | Limited model support, cloud-only | Growing | Latency-critical applications |

---

### Example 3: M&A Signal Detection

**Scenario**: Detect early acquisition indicators for companies in the enterprise observability space (Datadog, Grafana Labs, Chronosphere, Honeycomb).

**Step 1 — Define M&A signal categories**

| Signal Category | Indicators | Weight |
|----------------|-----------|--------|
| Executive changes | CEO/CFO departure, new "Chief Strategy Officer", board additions | High |
| Hiring patterns | Sudden corporate development/M&A roles, legal team expansion | High |
| Financial signals | Unusual funding, secondary sales, down round, runway concerns | High |
| Strategic moves | Exclusive partnerships, technology licensing, IP transfers | Medium |
| Market behavior | Quiet period (no product updates), website changes, domain changes | Medium |
| Social signals | Founder tone shifts, "exciting news soon" posts, unusual silence | Low |

**Step 2 — Build targeted queries**

```
"Chronosphere" AND ("acquisition" OR "acquire" OR "acqui-hire" OR "merger")
"Honeycomb" AND ("strategic alternatives" OR "exploring options" OR "advisors")
"Grafana Labs" AND ("corporate development" OR "M&A" OR "strategic partnership")
site:linkedin.com "Chronosphere" "corporate development" OR "M&A"
site:sec.gov "Honeycomb" OR "Hound Technology"
"[company]" "quiet period" OR "exciting announcement"
"[company]" hiring "corporate development" OR "business development director"
"[company]" board of directors new appointment
```

**Step 3 — Entity and event extraction**

From collected sources, extract and classify:

```json
[
  {
    "type": "event",
    "name": "Chronosphere CFO departure",
    "date": "2025-10-28",
    "entities": ["Chronosphere", "Lisa Park"],
    "signal_category": "executive_change",
    "m_and_a_weight": "high",
    "source": "linkedin.com/posts/lisapark-farewell"
  },
  {
    "type": "event",
    "name": "Honeycomb hires Goldman Sachs advisor",
    "date": "2025-11-02",
    "entities": ["Honeycomb", "Goldman Sachs"],
    "signal_category": "financial",
    "m_and_a_weight": "high",
    "source": "theinformation.com/articles/honeycomb-advisors"
  },
  {
    "type": "event",
    "name": "Datadog acquires incident.io",
    "date": "2025-11-08",
    "entities": ["Datadog", "incident.io"],
    "signal_category": "strategic",
    "m_and_a_weight": "confirmed_event",
    "source": "datadog.com/blog/incident-io-acquisition"
  }
]
```

**Step 4 — Score composite M&A probability**

Aggregate signals per company over a rolling 90-day window:

```
Chronosphere:
  - CFO departed (high)         +3
  - 2 corp dev job postings     +2
  - No product release in 90d   +1
  - Composite score: 6/10 → ELEVATED

Honeycomb:
  - Hired investment bank       +4
  - Board added PE partner      +2
  - Founder "grateful" post     +1
  - Composite score: 7/10 → HIGH

Grafana Labs:
  - New enterprise partnerships +1
  - Active hiring across all    -1 (normal growth, reduces M&A signal)
  - Composite score: 0/10 → LOW
```

**Step 5 — Generate M&A signal alert**

```markdown
# M&A Signal Alert: Enterprise Observability Sector
**Date**: 2025-11-10 | **Window**: 90 days

## HIGH probability
- **Honeycomb**: Investment bank engagement + board changes suggest active process.
  Key evidence: Goldman Sachs advisory (The Information), new PE board member.
  Likely acquirers: Datadog, Cisco, ServiceNow.

## ELEVATED probability
- **Chronosphere**: Leadership turnover + hiring freeze + corp dev roles.
  Key evidence: CFO departure, no product releases, corp dev postings on LinkedIn.
  Could indicate: acquisition target OR internal restructuring.

## LOW probability
- **Grafana Labs**: Normal operating patterns, active hiring, regular releases.
- **Datadog**: Active acquirer (incident.io deal closed), not a target.
```

---

## Advanced Entity Extraction

### Relationship Mapping from Unstructured Text

Extract relationships by identifying sentence-level patterns that connect two named entities.

**Pattern templates**:
```
[Person] joined [Company] as [Role]
  → relation: works_at, attributes: {role: Role, event: "joined"}

[Company] acquired [Company] for [Amount]
  → relation: acquired, attributes: {amount: Amount}

[Person] and [Person] co-founded [Company]
  → relations: founded (x2), co_founded_with (between persons)

[Company] partnered with [Company] to [Purpose]
  → relation: partnered_with, attributes: {purpose: Purpose}

[Person] left [Company] to join [Company]
  → relation: left (old), works_at (new), attributes: {event: "departure"}
```

**Multi-hop relationships**: When A relates to B and B relates to C, infer indirect connections:
```
Sarah Chen works_at BetaStack
BetaStack competes_with AlphaCloud
→ Indirect: Sarah Chen is key_person_at competitor of AlphaCloud
```

**Negation detection**: Watch for negated relationships that should NOT be added:
```
"Company X denied it was in acquisition talks with Company Y"
→ Do NOT add acquired relation. Add entity note: "denied acquisition rumor, [date]"

"Former CEO of Company X" → Person left. Mark works_at as ended.
```

### Temporal Event Extraction (Timeline Construction)

Extract dates and temporal markers to build event timelines.

**Explicit dates**:
```
"On March 15, 2025, Acme launched ProductX"
  → event: product_launch, date: 2025-03-15, entities: [Acme, ProductX]
```

**Relative dates** (resolve against article publication date):
```
"last week"     → pub_date - 7 days
"earlier today" → pub_date
"next quarter"  → pub_date + next fiscal quarter boundary
"in Q3"         → July-September of article's year
"recently"      → pub_date - 30 days (approximate, confidence: medium)
```

**Temporal ordering heuristics**:
```
"before the acquisition" → event precedes known acquisition date
"following the launch"   → event follows known launch date
"amid layoffs"          → event concurrent with layoff period
```

**Timeline output format**:
```json
{
  "entity": "Acme Corp",
  "timeline": [
    {"date": "2025-01-15", "event": "Series B ($40M)", "type": "funding", "confidence": "high"},
    {"date": "2025-03-20", "event": "Hired new CTO (Jane Lee)", "type": "leadership", "confidence": "high"},
    {"date": "2025-06-01", "event": "Launched v3.0", "type": "product", "confidence": "high"},
    {"date": "2025-08-10", "event": "Partnership with CloudCo", "type": "partnership", "confidence": "medium"},
    {"date": "2025-11-05", "event": "Acquired by BigCorp", "type": "acquisition", "confidence": "high"}
  ]
}
```

### Quantitative Data Extraction

Extract numerical data points with units, context, and time reference.

**Financial figures**:
```
Pattern: "[Company] raised $[amount][M/B] in [round]"
Example: "Acme raised $40M in Series B"
  → {metric: "funding", value: 40000000, currency: "USD", context: "Series B", entity: "Acme"}

Pattern: "[Company] revenue of $[amount][M/B]"
Example: "reported annual revenue of $120M"
  → {metric: "revenue", value: 120000000, currency: "USD", period: "annual", entity: subject}
```

**Growth rates**:
```
Pattern: "[metric] grew [X]% [period]"
Example: "ARR grew 45% year-over-year"
  → {metric: "ARR_growth", value: 0.45, period: "YoY", entity: subject}

Pattern: "from [X] to [Y]"
Example: "headcount grew from 200 to 350"
  → {metric: "headcount", previous: 200, current: 350, growth: 0.75, entity: subject}
```

**Headcounts and scale metrics**:
```
"[Company] now has [N] employees"
"[Company] serves [N] customers"
"[Product] has [N] monthly active users"
"[Company] operates in [N] countries"
```

**Extraction validation rules**:
- Currency amounts without a clear entity reference: discard or mark confidence "low"
- Growth percentages without a base period: mark confidence "medium"
- Round numbers (e.g., "about 1,000 employees"): flag as approximate
- Conflicting numbers from different sources: record both, note discrepancy

### Multi-Source Entity Resolution

When the same entity appears across different sources with variations, deduplicate.

**Company name normalization**:
```
"Acme Corp" = "Acme Corporation" = "Acme, Inc." = "ACME" (when context matches)
"Google" = "Alphabet" (parent) — but keep as separate entities with parent_of relation
```

**Resolution rules**:
| Signal | Match Confidence | Action |
|--------|-----------------|--------|
| Exact name match | High | Merge immediately |
| Name + same industry + same location | High | Merge |
| Abbreviated name + same context | Medium | Merge with note |
| Similar name, different industry | Low | Keep separate, flag for review |
| Person same name, different company | Low | Keep separate unless linked by career event |

**Deduplication process**:
1. Normalize: lowercase, strip legal suffixes, expand abbreviations
2. Match: compare against existing entity list using normalized form
3. Verify: check at least one corroborating attribute (industry, location, person association)
4. Merge: combine attributes, keep all source references, use highest confidence level
5. Log: record the merge decision for audit

```json
{
  "canonical": "entity_acme_corp",
  "aliases": ["Acme Corp", "Acme Corporation", "Acme, Inc.", "ACME"],
  "merged_from": ["source_techcrunch_entity_12", "source_linkedin_entity_89"],
  "merge_confidence": "high",
  "merge_reason": "exact name + same industry (SaaS) + same HQ (San Francisco)"
}
```

---

## Collection Automation Patterns

### Scheduled Collection Workflows

Define collection cadences matched to intelligence needs.

**Daily cycle** (for active competitive monitoring):
```
06:00 UTC — Run news queries for all targets (surface scan)
06:15 UTC — Check social media and forums for overnight mentions
06:30 UTC — Compare against yesterday's snapshot, flag changes
06:45 UTC — Generate daily brief, send alerts for CRITICAL items
```

**Weekly cycle** (for technology landscape and market mapping):
```
Monday  — Full source sweep: news, blogs, official sites
Tuesday — Job board scan: new postings, closed postings, pattern analysis
Wednesday — Financial data: funding rounds, SEC filings, earnings
Thursday — Community signals: GitHub activity, forum discussions, reviews
Friday  — Synthesis: generate weekly report, update entity graph, adjust queries
```

**Event-triggered cycle** (supplement scheduled runs):
```
Trigger: CRITICAL change detected in any cycle
  → Immediately run deep collection on the affected entity
  → Expand query set to cover related entities
  → Generate ad-hoc alert report
  → Shorten next scheduled cycle interval (e.g., weekly → daily for 7 days)
```

### Source Prioritization Based on Hit Rate

Track which sources consistently produce actionable intelligence and allocate collection effort accordingly.

**Hit rate calculation**:
```
hit_rate = (data_points_extracted / fetches_from_source) over last 10 cycles
```

**Priority tiers**:
| Hit Rate | Priority | Collection Behavior |
|----------|----------|-------------------|
| > 60% | Tier 1 | Always fetch, process first |
| 30-60% | Tier 2 | Fetch on every cycle |
| 10-30% | Tier 3 | Fetch every other cycle |
| < 10% | Tier 4 | Fetch weekly regardless of cycle frequency |
| 0% for 5+ cycles | Drop | Remove from active source list, log reason |

**Source performance tracking**:
```json
{
  "source": "techcrunch.com",
  "total_fetches": 48,
  "data_points_extracted": 31,
  "hit_rate": 0.65,
  "tier": 1,
  "avg_confidence": "medium-high",
  "last_hit": "2025-11-15",
  "best_queries": ["[company] funding", "[company] acquisition"]
}
```

### Incremental Collection (Only New/Changed Content)

Avoid re-processing unchanged content across cycles.

**Techniques**:
1. **URL deduplication**: Maintain a set of already-processed URLs. Skip on subsequent cycles.
2. **Content hashing**: Hash the extracted text body. If hash matches previous cycle, skip processing.
3. **Date filtering**: Append date ranges to queries to limit results to new content.
4. **Pagination cursors**: For APIs and structured sources, store the last-seen ID or timestamp.

**Query date narrowing**:
```
Cycle runs daily at 06:00 UTC:
  "AlphaCloud" after:2025-11-15 before:2025-11-16
  "AlphaCloud" news past 24 hours

Cycle runs weekly:
  "AlphaCloud" after:2025-11-08 before:2025-11-15
```

**State tracking for incremental collection**:
```json
{
  "processed_urls": ["https://example.com/article-1", "..."],
  "content_hashes": {"url1": "sha256:abc123", "url2": "sha256:def456"},
  "last_collection_time": "2025-11-15T06:00:00Z",
  "query_cursors": {
    "techcrunch_rss": "2025-11-15T05:30:00Z",
    "github_api_events": "event_id_98765"
  }
}
```

### Alert Trigger Conditions and Escalation Rules

Define when and how to escalate detected changes.

**Trigger conditions**:
```
IMMEDIATE ALERT (publish event_publish within the cycle):
  - Leadership change at target company (CEO, CTO, CFO)
  - Acquisition or merger announcement
  - Funding round > $10M
  - Product discontinuation or major pivot
  - Regulatory action or legal filing
  - Data breach or security incident

DAILY DIGEST (batch into next daily report):
  - New product feature or version release
  - New partnership announcement
  - Hiring surge (> 5 new roles in a category)
  - Pricing or packaging change
  - Significant sentiment shift (score delta > 2 in one cycle)

WEEKLY SUMMARY (include in weekly report only):
  - Blog posts and thought leadership
  - Conference appearances
  - Minor version updates or patches
  - Individual job postings
  - Social media activity within normal range
```

**Escalation rules**:
```
Level 1 — Auto-include in next scheduled report (default for all changes)
Level 2 — event_publish immediately (for CRITICAL significance changes)
Level 3 — event_publish + re-run deep collection on affected entity (for M&A, major crises)
```

**False positive suppression**:
- Require 2+ independent sources before triggering Level 2 alerts
- Ignore "rumor" or "speculation" tagged content for immediate alerts
- If the same alert fired in the previous cycle with no new corroboration, suppress repeat

---

## Analysis Techniques

### Link Analysis (Connection Mapping)

Map the network of relationships between entities to reveal hidden connections, influence patterns, and structural vulnerabilities.

**Building the adjacency map**:
```
From the knowledge graph, extract all relations and build:

Nodes: [Acme, BetaCo, GammaSuite, Jane Lee, CloudCo, InvestorX]
Edges:
  Acme       --competes_with-->    BetaCo
  Acme       --partnered_with-->   CloudCo
  Jane Lee   --works_at-->         Acme
  Jane Lee   --formerly-->         BetaCo
  InvestorX  --invested_in-->      Acme
  InvestorX  --invested_in-->      GammaSuite
```

**Key metrics to compute**:
| Metric | Meaning | Use |
|--------|---------|-----|
| Degree centrality | Number of direct connections | Identifies most-connected entities |
| Shared connections | Entities with overlapping relationships | Reveals indirect competition or collaboration |
| Bridge nodes | Entities connecting otherwise separate clusters | Identifies key influencers or gatekeepers |
| Cluster density | Ratio of actual to possible connections in a group | Measures how tightly coupled a set of entities is |

**Practical analysis patterns**:
```
Investor overlap:
  InvestorX invested_in Acme AND GammaSuite
  → Potential: board-level information sharing, future merger pressure

Talent flow:
  Jane Lee: BetaCo (2020-2024) → Acme (2024-present)
  3 other engineers: BetaCo → Acme in same period
  → Pattern: talent drain from BetaCo to Acme, possible IP risk

Supply chain dependency:
  Acme uses CloudCo infrastructure
  BetaCo uses CloudCo infrastructure
  → Shared dependency: CloudCo outage affects both competitors
```

### Timeline Analysis (Event Sequencing and Pattern Detection)

Arrange extracted events chronologically to detect causal chains, recurring patterns, and anomalous timing.

**Constructing the timeline**:
```
2025-01  Acme raises Series B ($40M)
2025-02  Acme posts 15 engineering roles
2025-03  Acme hires CTO from Google
2025-05  Acme acquires small startup (data pipeline tool)
2025-06  Acme launches v3.0 with data pipeline features
2025-08  Acme announces enterprise pricing tier
```

**Pattern detection rules**:

| Pattern | Sequence | Interpretation |
|---------|----------|---------------|
| Build-up to launch | Funding → Hiring surge → Leadership hire → Product release | Normal growth execution |
| Acquisition integration | Acquire company → Quiet period (2-4 months) → Feature launch using acquired tech | Successful integration |
| Pre-acquisition signals | Advisor hire → Leadership departures → Quiet period → Announcement | Target company being acquired |
| Distress pattern | Layoffs → Pricing cuts → Leadership change → Pivot or shutdown | Company in trouble |
| Expansion play | Funding → New market entry → Localized hiring → Regional partnerships | Geographic or vertical expansion |

**Anomaly detection**:
```
Expected: Funding round → hiring surge within 60 days
Observed: Funding round → no hiring after 90 days
→ Flag: "Post-funding hiring anomaly — possible pivot, internal issues, or stealth project"

Expected: Product launch → marketing push within 30 days
Observed: Product launch → silence
→ Flag: "Launch without marketing — possible soft launch, or product issues"
```

### Trend Detection (Acceleration, Deceleration, Inflection Points)

Track metrics across collection cycles to identify directional shifts.

**Metric tracking format**:
```json
{
  "entity": "Acme Corp",
  "metric": "job_postings",
  "series": [
    {"cycle": 1, "date": "2025-09-01", "value": 12},
    {"cycle": 2, "date": "2025-09-08", "value": 18},
    {"cycle": 3, "date": "2025-09-15", "value": 31},
    {"cycle": 4, "date": "2025-09-22", "value": 45},
    {"cycle": 5, "date": "2025-09-29", "value": 42}
  ]
}
```

**Trend classification**:
| Pattern | Detection Rule | Meaning |
|---------|---------------|---------|
| Accelerating | Growth rate increasing cycle-over-cycle | Expanding investment in area |
| Decelerating | Growth rate decreasing but still positive | Approaching saturation or shift in priorities |
| Inflection point | Direction change (growth → decline or vice versa) | Strategic shift, market event, or external shock |
| Plateau | Value stable within 10% for 3+ cycles | Steady state, maintenance mode |
| Spike | Single-cycle jump > 2x previous value | One-time event (launch, announcement, crisis) |
| Cliff | Single-cycle drop > 50% | Sudden change (layoff, shutdown, policy change) |

**Multi-metric correlation**:
```
When two metrics move together, the correlation strengthens the signal:

Acme job_postings: accelerating
Acme github_commits: accelerating
→ Corroborated signal: major development push underway

BetaCo job_postings: cliff (-60%)
BetaCo glassdoor_rating: declining
→ Corroborated signal: organizational distress
```

### Competitive Positioning Maps

Synthesize collected intelligence into comparative frameworks.

**Feature parity matrix**:
| Capability | Acme | BetaCo | GammaSuite | Your Product |
|-----------|------|--------|------------|-------------|
| Real-time dashboards | Yes (v2.0+) | Yes | Limited | Yes |
| AI-powered alerts | Yes (new in v4.2) | No | Beta | Planned Q1 |
| On-prem deployment | No | Yes | Yes | Yes |
| SOC2 compliance | Yes | Yes | In progress | Yes |
| Free tier | No | Yes (limited) | Yes | Yes |

**Market position quadrant** (based on collected metrics):
```
                    High Market Share
                         |
           Leaders       |      Challengers
          (Acme)         |      (GammaSuite)
                         |
  Low Growth ────────────┼──────────── High Growth
                         |
           Declining     |      Emerging
          (Legacy Co)    |      (BetaCo)
                         |
                    Low Market Share
```

Inputs for positioning:
- **Market share proxy**: mention frequency, customer count, job posting volume
- **Growth proxy**: funding recency, hiring rate, product release velocity, GitHub star velocity

**Pricing intelligence table**:
| Tier | Acme | BetaCo | GammaSuite | Notes |
|------|------|--------|------------|-------|
| Free | -- | 5 users | 10 users | BetaCo most restrictive |
| Team | $15/user/mo | $12/user/mo | $20/user/mo | BetaCo cheapest |
| Enterprise | Custom | $35/user/mo | Custom | BetaCo only one with public enterprise pricing |
| Notable changes | Raised Team tier 20% in Q3 | Unchanged 12 months | New tier added Q4 | Acme pricing pressure |

Track pricing changes across cycles — pricing increases signal confidence, decreases signal competitive pressure or churn concerns.
