---
name: researcher-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI deep research — methodology, source evaluation, search optimization, cross-referencing, synthesis, and citation formats"
runtime: prompt_only
---

# Deep Research Expert Knowledge

## Research Methodology

### Research Process (5 phases)
1. **Define**: Clarify the question, identify what's known vs unknown, set scope
2. **Search**: Systematic multi-strategy search across diverse sources
3. **Evaluate**: Assess source quality, extract relevant data, note limitations
4. **Synthesize**: Combine findings into coherent answer, resolve contradictions
5. **Verify**: Cross-check critical claims, identify remaining uncertainties

### Question Types & Strategies
| Question Type | Strategy | Example |
|--------------|----------|---------|
| Factual | Find authoritative primary source | "What is the population of Tokyo?" |
| Comparative | Multi-source balanced analysis | "React vs Vue for large apps?" |
| Causal | Evidence chain + counterfactuals | "Why did Theranos fail?" |
| Predictive | Trend analysis + expert consensus | "Will quantum computing replace classical?" |
| How-to | Step-by-step from practitioners | "How to set up a Kubernetes cluster?" |
| Survey | Comprehensive landscape mapping | "What are the options for vector databases?" |
| Controversial | Multiple perspectives + primary sources | "Is remote work more productive?" |

### Decomposition Technique
Complex questions should be broken into sub-questions:
```
Main: "Should our startup use microservices?"
Sub-questions:
  1. What are microservices? (definitional)
  2. What are the benefits vs monolith? (comparative)
  3. What team size/stage is appropriate? (contextual)
  4. What are the operational costs? (factual)
  5. What do similar startups use? (case studies)
  6. What are the migration paths? (how-to)
```

---

## CRAAP+ Source Evaluation Framework

### Standard CRAAP Criteria

**Currency**
- When was it published or last updated?
- Is the information still current for the topic?
- For technology topics: anything >2 years old may be outdated
- For science: check if the paper has been superseded by newer work

**Relevance**
- Does it directly address your question?
- Who is the intended audience?
- Is the level of detail appropriate?

**Authority**
- Who is the author? What are their credentials in this specific domain?
- What institution published this?
- Does the URL domain indicate authority? (.gov, .edu, reputable org)
- Is this person's authority relevant to the claim? (A Nobel physicist is not an authority on epidemiology)

**Accuracy**
- Is the information supported by evidence?
- Has it been reviewed or refereed?
- Can you verify the claims from other sources?
- Are there factual errors, typos, or broken logic?

**Purpose**
- Why does this information exist?
- Is it informational, commercial, persuasive, or entertainment?
- Does the author/organization benefit financially or politically from you believing this?

### Advanced Evaluation (CRAAP+ Extensions)

Apply these additional checks for thorough/exhaustive research:

**Methodological Rigor**
- Does the source describe its methodology? If empirical: what is the sample size, selection method, and study design?
- Are confounders acknowledged? Are limitations discussed?
- For surveys: what was the response rate? Is the sample representative?
- Red flag: a study that reports only favorable results with no limitations section

**Citation Chain Analysis**
- Does the source cite primary research, or only other secondary/tertiary sources?
- Follow the chain: if Source B cites Source A, read Source A directly. The original may say something different from how it was cited.
- "Citogenesis" check: multiple sources may all trace back to a single unverified claim (e.g., a Wikipedia edit that got cited by news articles that then got cited as "multiple sources confirm")

**Conflict of Interest Detection**
- Is the research funded by an entity with a stake in the outcome?
- Is the author affiliated with a company or lobby group related to the topic?
- Does the publication accept sponsored content without clear labeling?
- Example: a study finding "our product outperforms competitors" funded by the product vendor is not independent evidence

**Replication & Consensus Check**
- Has the finding been replicated by independent groups?
- Does it align with the broader expert consensus, or is it an outlier?
- If it contradicts consensus: does it provide a compelling methodological reason?

### Scoring
```
A (Authoritative):  Passes all CRAAP criteria + methodological rigor confirmed
B (Reliable):       Passes CRAAP, minor concern on one advanced check
C (Useful):         Passes 3/5 CRAAP, use with caveats noted
D (Weak):           Fails multiple criteria OR has unresolved COI
F (Unreliable):     Fails most criteria, do not cite
```

---

## Search Query Optimization

### Query Construction Techniques

**Exact phrase**: `"specific phrase"` — use for names, quotes, error messages
**Site-specific**: `site:domain.com query` — search within a specific site
**Exclude**: `query -unwanted_term` — remove irrelevant results
**File type**: `filetype:pdf query` — find specific document types
**Recency**: `query after:2024-01-01` — recent results only
**OR operator**: `query (option1 OR option2)` — broaden search
**Wildcard**: `"how to * in python"` — fill-in-the-blank

### Multi-Strategy Search Pattern
For each research question, use at least 3 search strategies:
1. **Direct**: The question as-is
2. **Authoritative**: `site:gov OR site:edu OR site:org [topic]`
3. **Academic**: `[topic] research paper [year]` or `site:arxiv.org [topic]`
4. **Practical**: `[topic] guide` or `[topic] tutorial` or `[topic] how to`
5. **Data**: `[topic] statistics` or `[topic] data [year]`
6. **Contrarian**: `[topic] criticism` or `[topic] problems` or `[topic] myths`

### Source Discovery by Domain
| Domain | Best Sources | Search Pattern |
|--------|-------------|---------------|
| Technology | Official docs, GitHub, Stack Overflow, engineering blogs | `[tech] documentation`, `site:github.com [tech]` |
| Science | PubMed, arXiv, Nature, Science | `site:arxiv.org [topic]`, `[topic] systematic review` |
| Business | SEC filings, industry reports, HBR | `[company] 10-K`, `[industry] report [year]` |
| Medicine | PubMed, WHO, CDC, Cochrane | `site:pubmed.ncbi.nlm.nih.gov [topic]` |
| Legal | Court records, law reviews, statute databases | `[case] ruling`, `[law] analysis` |
| Statistics | Census, BLS, World Bank, OECD | `site:data.worldbank.org [metric]` |
| Current events | Reuters, AP, BBC, primary sources | `[event] statement`, `[event] official` |

### Academic & Grey Literature Search Strategies

Not all valuable research is published in mainstream outlets. Grey literature (reports, theses, working papers, conference proceedings, preprints) often contains the most detailed and current findings.

**Academic databases and how to use them**:
```
Google Scholar    → Broad academic search. Use "cited by" to find follow-up work.
                    Check "Related articles" for adjacent findings.
arXiv.org         → CS, physics, math preprints. Free. NOT peer-reviewed — note this.
PubMed            → Biomedical/health. Use MeSH terms for precise queries.
SSRN              → Social science, economics, law working papers.
Semantic Scholar  → AI-enhanced academic search with citation graphs.
IEEE Xplore       → Engineering and CS papers (often paywalled — check for preprints).
```

**Grey literature sources by domain**:
```
Policy/government:  Government reports, GAO studies, parliamentary inquiries
                    → site:gao.gov, site:*.gov/reports, site:oecd.org
Think tanks:        Brookings, RAND, Chatham House, NBER
                    → "[topic] site:rand.org OR site:brookings.edu"
Industry reports:   Vendor-neutral analyst reports, trade association data
                    → "[topic] industry report filetype:pdf"
Theses:             University repositories (often the most detailed single-topic work)
                    → "[topic] thesis OR dissertation filetype:pdf site:*.edu"
Standards bodies:   NIST, ISO, W3C, IETF RFCs
                    → "[topic] site:nist.gov OR site:w3.org OR site:rfc-editor.org"
Conference proc.:   Slides and papers from domain-specific conferences
                    → "[topic] [conference name] proceedings OR slides"
```

**Citation chain technique**: When you find one highly relevant paper:
1. Read its references for foundational work (backward search)
2. Search "cited by" to find newer work that builds on it (forward search)
3. Check the authors' other publications for related work
4. This often uncovers sources that keyword searches miss

### Systematic Review Methodology (Lite)

For exhaustive-tier research, apply a lightweight systematic review approach:

1. **Define inclusion/exclusion criteria** before searching:
   - Date range, language, source types, geographic scope
   - What counts as "relevant" — define upfront, not after seeing results
2. **Document your search strategy**: record every query, database, and date searched
3. **Screen results in two passes**:
   - Pass 1: title and snippet — exclude obviously irrelevant results
   - Pass 2: read the full source — evaluate against inclusion criteria
4. **Extract data consistently**: use the same extraction template for every source
5. **Report the numbers**: "Searched N databases, retrieved M results, N1 passed screening, N2 included in final synthesis"

This is not a full academic systematic review, but it adds rigor and transparency that distinguishes exhaustive research from ad hoc searching.

---

## Cross-Referencing Techniques

### Verification Levels
```
Level 1: Single source (unverified)
  → Mark as "reported by [source]"

Level 2: Two independent sources agree (corroborated)
  → Mark as "confirmed by multiple sources"

Level 3: Primary source + secondary confirmation (verified)
  → Mark as "verified — primary source: [X]"

Level 4: Expert consensus (well-established)
  → Mark as "widely accepted" or "scientific consensus"
```

### Contradiction Resolution Decision Tree

When sources disagree, work through this structured process:

```
CONFLICT: Source A says X, Source B says Y
  │
  ├─ 1. Scope check: Are they measuring the same thing?
  │     Example: "React is faster" vs "Vue is faster" — one measures
  │     initial render, the other measures re-render. Not a real conflict.
  │     → If different scope: report both with context, not as a conflict.
  │
  ├─ 2. Quality gap: Compare CRAAP+ scores
  │     → If 2+ letter grades apart: favor higher-rated source, note the
  │       disagreement. Example: peer-reviewed study (A) vs blog post (C)
  │       on the same empirical question — favor the study.
  │
  ├─ 3. Temporal ordering: Is one an update/correction of the other?
  │     → If newer source explicitly addresses and corrects older data:
  │       favor newer. Example: "Our 2024 study corrects the methodology
  │       flaw in the 2022 paper" — favor 2024.
  │
  ├─ 4. Methodology comparison: Which has stronger evidence?
  │     Consider: sample size, study design (RCT > observational > anecdote),
  │     peer review status, replication.
  │     → Favor stronger methodology. Explain the methodological difference.
  │
  ├─ 5. Conflict of interest: Does one source have a COI?
  │     → Favor the source without COI. Disclose the COI explicitly.
  │     Example: vendor benchmark vs independent benchmark — favor independent.
  │
  ├─ 6. Consensus weight: What do other sources say?
  │     → If 5 sources say X and 1 credible source says Y: report X as
  │       the majority view, Y as a noted dissenting position.
  │
  └─ 7. Genuinely disputed: No resolution possible
        → Present both positions with full evidence. Mark as "Disputed."
        Do NOT force a conclusion. State what additional evidence would
        resolve the conflict.
```

### Source Independence Verification

Two articles citing the same original study are ONE source, not two:
- Trace every claim to its origin before counting source agreement
- News articles often rewrite the same press release — that is one source
- "Multiple outlets report" is not corroboration if they share a single upstream source
- Independent means: different data collection, different research team, different methodology

---

## Synthesis Patterns

### Source Triangulation

Before synthesizing, verify key claims through triangulation — confirming a finding via multiple independent evidence types:

```
Triangulation types:
  Data triangulation:     Same question examined with different datasets
  Method triangulation:   Same question studied with different methods
                          (e.g., survey + case study + statistical analysis)
  Source triangulation:   Same claim confirmed by sources with different
                          perspectives (e.g., vendor + customer + analyst)
  Temporal triangulation: Finding holds across different time periods
```

A claim supported by multiple triangulation types is much stronger than one confirmed by multiple sources of the same type. "Three blog posts agree" is weaker than "a blog post, a peer-reviewed study, and an SEC filing agree."

### Narrative Synthesis
```
The evidence suggests [main finding].

[Source A] found that [finding 1], which is consistent with
[Source B]'s observation that [finding 2]. However, [Source C]
presents a contrasting view: [finding 3].

The weight of evidence favors [conclusion] because [reasoning].
A key limitation is [gap or uncertainty].
```

### Structured Synthesis
```
FINDING 1: [Claim]
  Evidence for: [Source A], [Source B] — [details]
  Evidence against: [Source C] — [details]
  Triangulation: [data/method/source types used]
  Confidence: [high/medium/low]
  Reasoning: [why the evidence supports this finding]

FINDING 2: [Claim]
  ...
```

### Gap Analysis
After synthesis, explicitly note:
- What questions remain unanswered?
- What data would strengthen the conclusions?
- What are the limitations of the available sources?
- What follow-up research would be valuable?
- What types of triangulation are missing? (e.g., "All sources are practitioner blogs — no academic validation exists")

---

## Worked Examples

### Example 1: Technology Adoption Decision

**Question**: "Should our company adopt Rust for backend services?"

**Phase 1 — Define**

Decompose into sub-questions:
```
Main: "Should our company adopt Rust for backend services?"
Sub-questions:
  1. What are Rust's strengths for backend work? (factual)
  2. What are the real-world costs of adoption? (factual + case studies)
  3. How does Rust compare to our current stack (Go) on key metrics? (comparative)
  4. What do teams of our size (15-30 engineers) report? (case studies)
  5. What is the hiring/training landscape? (survey)
  6. What are the migration paths and risks? (how-to + risk analysis)
```

Scope constraints: Backend HTTP services, team of 20 engineers currently using Go, latency-sensitive workloads, 18-month planning horizon.

**Phase 2 — Search (multi-strategy)**
```
Strategy 1 (Direct):         "Rust backend production experience"
Strategy 2 (Authoritative):  site:arxiv.org "Rust" "memory safety" performance
Strategy 3 (Practical):      "migrating from Go to Rust" blog OR postmortem
Strategy 4 (Contrarian):     "Rust backend" problems OR regret OR "not worth"
Strategy 5 (Data):           "Rust" "developer survey" adoption 2024 2025
Strategy 6 (Case studies):   site:engineering.*.com Rust adoption
```

**Phase 3 — Evaluate (CRAAP scoring)**
```
Source 1: Rust annual survey (rust-lang.org)          → A (primary, current)
Source 2: Discord engineering blog on Rust migration   → A (primary, practitioner)
Source 3: Figma "Rust in production" post              → A (primary, detailed metrics)
Source 4: Random Medium post "Rust is the future"      → D (no credentials, no data)
Source 5: AWS SDK for Rust announcement                → B (authoritative, but marketing)
Source 6: "Why we moved back to Go" blog post          → B (primary experience, single case)
Source 7: Stack Overflow developer survey              → A (large sample, methodology documented)
```

Drop Source 4 entirely. Use Source 6 as a counterpoint despite being a single case.

**Phase 4 — Synthesize**
```
FINDING 1: Rust delivers measurable performance and reliability gains
  Evidence for: Discord reported 50% memory reduction after migration [2].
    Figma measured p99 latency improvements of 3-5x for compute-heavy paths [3].
  Evidence against: Gains may be marginal for I/O-bound CRUD services [6].
  Confidence: High for compute-intensive workloads, medium for I/O-bound.

FINDING 2: Adoption cost is front-loaded and significant
  Evidence for: Average ramp-up time for experienced Go/C++ engineers is
    3-6 months to productive Rust [2][7]. Compile times 2-5x longer than Go [3].
  Evidence against: Teams report that after the learning curve, maintenance
    costs drop due to fewer production incidents [2][3].
  Confidence: High

FINDING 3: Hiring pipeline is narrow but growing
  Evidence for: Rust ranks as "most admired" language for 8 consecutive years
    in SO survey, but only ~13% of developers use it professionally [7].
  Evidence against: Rust job demand is growing ~40% YoY [7].
  Confidence: Medium — hiring data is self-reported.
```

**Phase 5 — Verify and deliver**

Cross-check: Discord and Figma metrics are confirmed by independent engineering talks. SO survey methodology is published and peer-reviewed.

Final recommendation structure:
```
Adopt for: Latency-sensitive, compute-heavy services (strong evidence)
Avoid for: Simple CRUD APIs where Go is already performant (low ROI)
Mitigate hiring risk: Invest in internal training, start with one team
Timeline: 6-month pilot on a non-critical service before broader adoption
Confidence: Medium-high — strong technical evidence, moderate organizational evidence
```

### Example 2: Incident Analysis

**Question**: "What caused the 2024 CrowdStrike outage and what are the implications?"

**Phase 1 — Define**

This is a causal question with survey elements. Decompose:
```
Main: "What caused the 2024 CrowdStrike outage?"
Sub-questions:
  1. What happened? (timeline — factual)
  2. What was the technical root cause? (causal)
  3. What was the scope of impact? (factual, data)
  4. How did CrowdStrike respond? (factual)
  5. What systemic issues does this reveal? (analytical)
  6. What changed in the industry as a result? (survey + predictive)
```

**Phase 2 — Search**
```
Strategy 1 (Primary):     site:crowdstrike.com "July 2024" postmortem OR incident
Strategy 2 (Technical):   "CrowdStrike" "channel file" root cause analysis
Strategy 3 (Impact data): "CrowdStrike outage" damages OR cost OR impact 2024
Strategy 4 (Regulatory):  site:gov "CrowdStrike" review OR hearing OR testimony
Strategy 5 (Contrarian):  "CrowdStrike" "kernel driver" criticism before:2024-07-01
Strategy 6 (Expert):      "CrowdStrike outage" analysis site:*.edu OR site:arxiv.org
```

Note Strategy 5: searching for pre-incident criticism establishes whether warnings existed.

**Phase 3 — Evaluate and build timeline**
```
Timeline (verified — Level 3):
  2024-07-19 04:09 UTC  CrowdStrike deploys Channel File 291 update
  2024-07-19 04:09-05:27  Falcon sensor crashes → Windows BSOD on boot
  2024-07-19 05:27 UTC  CrowdStrike reverts the channel file
  2024-07-19 ~06:00     Scope becomes apparent: 8.5M Windows devices affected
  2024-07-19-21          Manual remediation required (boot to Safe Mode, delete file)
  2024-07-20-25          Airlines, hospitals, banks in multi-day recovery

Sources: CrowdStrike PIR [A], Microsoft blog [A], Reuters reporting [B],
  Congressional testimony transcript [A]
```

**Phase 4 — Synthesize root cause**
```
FINDING 1: Technical root cause was an out-of-bounds memory read
  A channel file update (type 291) contained malformed data.
  The Falcon sensor's Content Interpreter triggered an OOB read,
  causing a kernel-level crash (BSOD). The sensor ran as a kernel
  driver, so its crash took down the entire OS.
  Sources: CrowdStrike PIR [A], independent reverse engineering [B]
  Confidence: High (confirmed by vendor + independent analysis)

FINDING 2: The update bypassed adequate testing
  Channel files ("rapid response content") used a different validation
  pipeline than sensor code. The Template Type tested had 20 input
  fields; the deployed content provided 21. The validator did not
  catch the mismatch.
  Sources: CrowdStrike PIR [A], Congressional testimony [A]
  Confidence: High

FINDING 3: Impact — $5-10B+ in estimated damages
  8.5M devices affected (Microsoft estimate). Delta Air Lines alone
  reported $500M in losses. Parametrix estimated $5.4B in direct
  losses for Fortune 500 companies.
  Sources: Microsoft [A], Parametrix [B], Delta SEC filing [A]
  Confidence: Medium-high (total figure is estimated, individual claims are documented)

FINDING 4: Systemic issue — monoculture risk in security infrastructure
  A single vendor's kernel-level agent was present on ~24% of
  enterprise Windows endpoints. Pre-incident criticism of kernel-mode
  security agents existed but was not widely acted upon.
  Sources: Congressional hearing [A], pre-incident security research [B]
  Confidence: High
```

**Phase 5 — Verify and present implications**
```
Verified implications (cross-referenced across 3+ independent sources):
  1. Regulatory pressure on kernel-mode security agents accelerated
  2. Microsoft announced Windows Resiliency Initiative (user-mode alternatives)
  3. Enterprise customers began requiring staged/canary rollout for security updates
  4. Cyber insurance models updated to account for single-vendor concentration

Remaining uncertainties:
  - Full financial impact is still in litigation (Delta v. CrowdStrike)
  - Long-term market share impact on CrowdStrike is unclear
  - Whether kernel-mode restrictions will actually be enforced
```

---

## Citation Formats

### Inline URL
```
According to a 2024 study (https://example.com/study), the effect was significant.
```

### Footnotes
```
According to a 2024 study[1], the effect was significant.

---
[1] https://example.com/study — "Title of Study" by Author, Published Date
```

### Academic (APA)
```
In-text: (Smith, 2024)
Reference: Smith, J. (2024). Title of the article. *Journal Name*, 42(3), 123-145. https://doi.org/10.xxxx
```

For web sources (APA):
```
Author, A. A. (Year, Month Day). Title of page. Site Name. https://url
```

### Numbered References
```
According to recent research [1], the finding was confirmed by independent analysis [2].

## References
1. Author (Year). Title. URL
2. Author (Year). Title. URL
```

---

## Output Templates

### Brief Report
```markdown
# [Question]
**Date**: YYYY-MM-DD | **Sources**: N | **Confidence**: high/medium/low

## Answer
[2-3 paragraph direct answer]

## Key Evidence
- [Finding 1] — [source]
- [Finding 2] — [source]
- [Finding 3] — [source]

## Caveats
- [Limitation or uncertainty]

## Sources
1. [Source](url)
2. [Source](url)
```

### Detailed Report
```markdown
# Research Report: [Question]
**Date**: YYYY-MM-DD | **Depth**: thorough | **Sources Consulted**: N

## Executive Summary
[1 paragraph synthesis]

## Background
[Context needed to understand the findings]

## Methodology
[How the research was conducted, what was searched, how sources were evaluated]

## Findings

### [Sub-question 1]
[Detailed findings with inline citations]

### [Sub-question 2]
[Detailed findings with inline citations]

## Analysis
[Synthesis across findings, patterns identified, implications]

## Contradictions & Open Questions
[Areas of disagreement, gaps in knowledge]

## Confidence Assessment
[Overall confidence level with reasoning]

## Sources
[Full bibliography in chosen citation format]
```

---

## Cognitive Bias Detection & Countermeasures

These biases are not hypothetical — they actively distort research outcomes. For each bias below, apply the countermeasure as a concrete step in your process.

### 1. Confirmation Bias
**What it is**: Favoring information that confirms your initial hypothesis while unconsciously discounting contradictory evidence.
**How it manifests in research**: You find 3 sources supporting your initial hunch and stop searching. You dismiss a contradicting source as "low quality" without rigorous evaluation.
**Countermeasure**: In Phase 1, write down your initial assumption explicitly. In Phase 2, construct at least one "contrarian query" specifically designed to find disconfirming evidence. In Phase 4, count your sources: if >80% support one side, force a targeted search for the opposing view.
**Example**: Researching "Is TypeScript worth adopting?" — if your first 5 sources all say yes, search specifically for "TypeScript problems", "TypeScript not worth it", "TypeScript migration regret".

### 2. Anchoring Bias
**What it is**: The first piece of information you encounter disproportionately shapes your entire analysis.
**How it manifests in research**: The first article frames the topic in a specific way, and subsequent research unconsciously filters through that frame.
**Countermeasure**: After gathering all sources, re-read your synthesis. Ask: "Would I have written this the same way if I had encountered Source N first instead of Source 1?" If the first source you read is still dominating the framing, consciously rewrite the synthesis from a different source's perspective and compare.

### 3. Availability Bias
**What it is**: Over-weighting information that is easy to find (top search results, English-language, well-promoted content).
**Countermeasure**: After initial searches, ask: "What voices are missing?" Consider: non-English sources, academic papers behind paywalls (check preprint servers), practitioner experience that does not get blog posts (failure stories are under-reported). For exhaustive research, explicitly search grey literature and non-English sources.

### 4. Survivorship Bias
**What it is**: Only seeing successes because failures are invisible — they do not publish blog posts or get media coverage.
**How it manifests in research**: Technology X looks universally successful because companies that failed with it quietly moved on without writing about it.
**Countermeasure**: For any "should we adopt X?" question, explicitly search for: "[X] failure", "[X] abandoned", "[X] migration away from", "[X] post-mortem". Check GitHub for projects that started with X and switched away (look at archived repos, migration PRs).
**Example**: Researching microservices adoption — searching only for success stories will miss the many companies that reverted to monoliths but did not publicize it.

### 5. Authority Bias
**What it is**: Deferring to prestigious sources even when their evidence is thin.
**Countermeasure**: Evaluate the evidence, not the letterhead. A well-designed study from an unknown university with n=10,000 outweighs an opinion piece in a famous journal. Check: does the prestigious source provide data, or just assertions? Would you accept this evidence if it came from an unknown author?

### 6. Framing Bias
**What it is**: Being influenced by how data is presented rather than what the data shows.
**How it manifests in research**: "90% success rate" vs "10% failure rate" — same data, different impression. Relative vs absolute risk: "doubles the risk" could mean 0.001% to 0.002%.
**Countermeasure**: When a source presents a statistic, mentally reframe it: convert relative to absolute numbers, invert percentages, check base rates. If a claim sounds dramatic, check the absolute magnitude.

---

## Domain-Specific Research Tips

### Technology Research
- Always check the official documentation first
- Compare documentation version with the latest release
- Stack Overflow answers may be outdated — check the date
- GitHub issues/discussions often have the most current information
- Benchmarks without methodology descriptions are unreliable

### Business Research
- SEC filings (10-K, 10-Q) are the most reliable public company data
- Press releases are marketing — verify claims independently
- Analyst reports may have conflicts of interest — check disclaimers
- Employee reviews (Glassdoor) provide internal perspective but are biased

### Scientific Research
- Systematic reviews and meta-analyses are strongest evidence
- Single studies should not be treated as definitive
- Check if findings have been replicated
- Preprints have not been peer-reviewed — note this caveat
- p-values and effect sizes both matter — not just "statistically significant"

---

## Research Shortcuts

### When to Stop Researching

Research has diminishing returns. Recognize these signals:

**Stop signals — you have enough**:
- Three independent sources converge on the same answer
- New searches return sources you have already seen
- The last 3 searches added no new information or perspectives
- You have found primary source data that directly answers the question
- Remaining disagreements are about edge cases, not the core finding

**Keep going signals — you do not have enough**:
- Only one source supports a critical claim
- Two credible sources directly contradict each other with no resolution
- The requester's specific context (industry, scale, constraints) is not addressed
- You have secondary reporting but no primary source for a key fact
- Your confidence assessment would be "low" on a central finding

**Time-boxing rule**: For a standard research question, allocate effort roughly as:
```
Quick facts:       2-4 searches, 1-2 minutes
Standard question: 6-12 searches, 5-10 minutes
Deep dive:         15-30 searches, 20-40 minutes
```
If you exceed 2x the expected searches without convergence, stop and report what you have with explicit gaps noted.

### Quick Assessment vs Deep Dive

Not every question deserves a full 5-phase research process. Use this decision matrix:

```
Quick assessment (skip to synthesis fast):
  ✓ Question has a single factual answer
  ✓ Authoritative primary source exists and is accessible
  ✓ Low stakes — wrong answer has minimal consequences
  ✓ Requester wants speed over thoroughness
  Example: "What version of Python dropped GIL?"
    → Check python.org docs/PEPs, answer in one search.

Standard research (full 5-phase process):
  ✓ Comparative or analytical question
  ✓ Multiple valid perspectives exist
  ✓ Answer will inform a decision
  ✓ Moderate stakes
  Example: "React vs Svelte for our new dashboard?"
    → Full decomposition, multi-source, synthesis needed.

Deep dive (extended research with formal deliverable):
  ✓ High-stakes decision (architecture, vendor, strategy)
  ✓ Conflicting information is likely
  ✓ Historical context and trend analysis needed
  ✓ Requester expects a report they can share with others
  Example: "Should we move from AWS to multi-cloud?"
    → Multiple sub-questions, 10+ sources, formal report.
```

### Source Reuse Patterns

Not every question starts from zero. Build efficiency by recognizing reusable sources.

**Tier 1 — Canonical references (always check first for their domain)**:
```
Programming languages: Official docs, language spec, release notes
Cloud services:        AWS/GCP/Azure docs, status pages, pricing pages
Security:              CVE databases, vendor advisories, NIST NVD
Statistics:            Official census/survey data, World Bank, OECD
Companies:             SEC filings (EDGAR), official IR pages
Open source:           GitHub repo, CHANGELOG, issue tracker
```

**Tier 2 — High-signal aggregators (good starting points)**:
```
Technology trends:     ThoughtWorks Radar, Stack Overflow survey, TIOBE
Security incidents:    CISA advisories, Krebs on Security
Academic papers:       Google Scholar, Semantic Scholar, arXiv
Industry analysis:     Gartner (with bias caveat), a16z, Sequoia
Developer experience:  JetBrains survey, GitHub Octoverse
```

**Tier 3 — Practitioner sources (for real-world validation)**:
```
Engineering blogs:     Company engineering blogs (Netflix, Uber, Stripe, Discord)
Conference talks:      Recorded talks from Strange Loop, QCon, KubeCon
Community discussion:  Hacker News (comments often more valuable than articles),
                       Reddit (r/programming, r/devops, domain-specific subs)
```

**Anti-patterns to avoid**:
- Do not reuse a source across topics just because it scored well once — re-evaluate CRAAP for the new topic
- Do not treat aggregator rankings (Gartner Magic Quadrant, G2 reviews) as primary evidence — they are influenced by vendor spending
- Do not assume a source's authority transfers across domains — a security vendor's blog is authoritative on threats but not on database performance
