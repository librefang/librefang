---
name: predictor-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI forecasting — superforecasting principles, signal taxonomy, confidence calibration, reasoning chains, and accuracy tracking"
runtime: prompt_only
---

# Forecasting Expert Knowledge

## Superforecasting Principles

Based on research by Philip Tetlock and the Good Judgment Project:

1. **Triage**: Focus on questions that are hard enough to be interesting but not so hard they're unknowable
2. **Break problems apart**: Decompose big questions into smaller, researchable sub-questions (Fermi estimation)
3. **Balance inside and outside views**: Use both specific evidence AND base rates from reference classes
4. **Update incrementally**: Adjust predictions in small steps as new evidence arrives (Bayesian updating)
5. **Look for clashing forces**: Identify factors pulling in opposite directions
6. **Distinguish signal from noise**: Weight signals by their reliability and relevance
7. **Calibrate**: Your 70% predictions should come true ~70% of the time
8. **Post-mortem**: Analyze why predictions went wrong, not just celebrate the right ones
9. **Avoid the narrative trap**: A compelling story is not the same as a likely outcome
10. **Collaborate**: Aggregate views from diverse perspectives

---

## Signal Taxonomy

### Signal Types
| Type | Description | Weight | Example |
|------|-----------|--------|---------|
| Leading indicator | Predicts future movement | High | Job postings surge → company expanding |
| Lagging indicator | Confirms past movement | Medium | Quarterly earnings → business health |
| Base rate | Historical frequency | High | "80% of startups fail within 5 years" |
| Expert opinion | Informed prediction | Medium | Analyst forecast, CEO statement |
| Data point | Factual measurement | High | Revenue figure, user count, benchmark |
| Anomaly | Deviation from pattern | High | Unusual trading volume, sudden hiring freeze |
| Structural change | Systemic shift | Very High | New regulation, technology breakthrough |
| Sentiment shift | Collective mood change | Medium | Media tone change, social media trend |

### Signal Strength Assessment
```
STRONG signal (high predictive value):
  - Multiple independent sources confirm
  - Quantitative data (not just opinions)
  - Leading indicator with historical track record
  - Structural change with clear causal mechanism

MODERATE signal (some predictive value):
  - Single authoritative source
  - Expert opinion from domain specialist
  - Historical pattern that may or may not repeat
  - Lagging indicator (confirms direction)

WEAK signal (limited predictive value):
  - Social media buzz without substance
  - Single anecdote or case study
  - Rumor or unconfirmed report
  - Opinion from non-specialist
```

---

## Confidence Calibration

### Probability Scale
```
95% — Almost certain (would bet 19:1)
90% — Very likely (would bet 9:1)
80% — Likely (would bet 4:1)
70% — Probable (would bet 7:3)
60% — Slightly more likely than not
50% — Toss-up (genuine uncertainty)
40% — Slightly less likely than not
30% — Unlikely (but plausible)
20% — Very unlikely (but possible)
10% — Extremely unlikely
5%  — Almost impossible (but not zero)
```

### Calibration Rules
1. NEVER use 0% or 100% — nothing is absolutely certain
2. If you haven't done research, default to the base rate (outside view)
3. Your first estimate should be the reference class base rate
4. Adjust from the base rate using specific evidence (inside view)
5. Typical adjustment: ±5-15% per strong signal, ±2-5% per moderate signal
6. If your gut says 80% but your analysis says 55%, trust the analysis

### Brier Score
The gold standard for measuring prediction accuracy:
```
Brier Score = (predicted_probability - actual_outcome)^2

actual_outcome = 1 if prediction came true, 0 if not

Perfect score: 0.0 (you're always right with perfect confidence)
Coin flip: 0.25 (saying 50% on everything)
Terrible: 1.0 (100% confident, always wrong)

Good forecaster: < 0.15
Average forecaster: 0.20-0.30
Bad forecaster: > 0.35
```

---

## Domain-Specific Source Guide

### Technology Predictions
| Source Type | Examples | Use For |
|-------------|---------|---------|
| Product roadmaps | GitHub issues, release notes, blog posts | Feature predictions |
| Adoption data | Stack Overflow surveys, NPM downloads, DB-Engines | Technology trends |
| Funding data | Crunchbase, PitchBook, TechCrunch | Startup success/failure |
| Patent filings | Google Patents, USPTO | Innovation direction |
| Job postings | LinkedIn, Indeed, Levels.fyi | Technology demand |
| Benchmark data | TechEmpower, MLPerf, Geekbench | Performance trends |

### Finance Predictions
| Source Type | Examples | Use For |
|-------------|---------|---------|
| Economic data | FRED, BLS, Census | Macro trends |
| Earnings | SEC filings, earnings calls | Company performance |
| Analyst reports | Bloomberg, Reuters, S&P | Market consensus |
| Central bank | Fed minutes, ECB statements | Interest rates, policy |
| Commodity data | EIA, OPEC reports | Energy/commodity prices |
| Sentiment | VIX, put/call ratio, AAII survey | Market mood |

### Geopolitics Predictions
| Source Type | Examples | Use For |
|-------------|---------|---------|
| Official sources | Government statements, UN reports | Policy direction |
| Think tanks | RAND, Brookings, Chatham House | Analysis |
| Election data | Polls, voter registration, 538 | Election outcomes |
| Trade data | WTO, customs data, trade balances | Trade policy |
| Military data | SIPRI, defense budgets, deployments | Conflict risk |
| Diplomatic signals | Ambassador recalls, sanctions, treaties | Relations |

### Climate Predictions
| Source Type | Examples | Use For |
|-------------|---------|---------|
| Scientific data | IPCC, NASA, NOAA | Climate trends |
| Energy data | IEA, EIA, IRENA | Energy transition |
| Policy data | COP agreements, national plans | Regulation |
| Corporate data | CDP disclosures, sustainability reports | Corporate action |
| Technology data | BloombergNEF, patent filings | Clean tech trends |
| Investment data | Green bond issuance, ESG flows | Capital allocation |

---

## Reasoning Chain Construction

### Template
```
PREDICTION: [Specific, falsifiable claim]

1. REFERENCE CLASS (Outside View)
   Base rate: [What % of similar events occur?]
   Reference examples: [3-5 historical analogues]

2. SPECIFIC EVIDENCE (Inside View)
   Signals FOR (+):
   a. [Signal] — strength: [strong/moderate/weak] — adjustment: +X%
   b. [Signal] — strength: [strong/moderate/weak] — adjustment: +X%

   Signals AGAINST (-):
   a. [Signal] — strength: [strong/moderate/weak] — adjustment: -X%
   b. [Signal] — strength: [strong/moderate/weak] — adjustment: -X%

3. SYNTHESIS
   Starting probability (base rate): X%
   Net adjustment: +/-Y%
   Final probability: Z%

4. KEY ASSUMPTIONS
   - [Assumption 1]: If wrong, probability shifts to [W%]
   - [Assumption 2]: If wrong, probability shifts to [V%]

5. RESOLUTION
   Date: [When can this be resolved?]
   Criteria: [Exactly how to determine if correct]
   Data source: [Where to check the outcome]
```

---

## Worked Examples

### Example 1: Corporate Acquisition

**Question**: "Will Acme Corp be acquired within 12 months?" (asked January 2025)

```
PREDICTION: Acme Corp (mid-cap SaaS, $2B market cap) will be acquired by January 2026

1. REFERENCE CLASS (Outside View)
   Base rate: ~5-7% of publicly traded mid-cap SaaS companies receive
   acquisition offers in any given 12-month period.
   Reference examples:
   - Splunk acquired by Cisco (2023) — similar scale, strategic buyer
   - Figma attempted acquisition by Adobe (2022) — regulatory block
   - Nuance acquired by Microsoft (2021) — vertical SaaS, strategic fit
   - Mandiant acquired by Google (2022) — security vertical
   - Cvent acquired by Blackstone (2021) — PE buyout at depressed valuation

   Starting probability: 6%

2. SPECIFIC EVIDENCE (Inside View)
   Signals FOR (+):
   a. Board hired Goldman Sachs as advisor (leaked filing)
      — strength: STRONG — adjustment: +20%
      (Companies that retain M&A advisors complete a transaction ~40% of the time)
   b. CEO sold 30% of personal holdings in Q4 (SEC filing)
      — strength: MODERATE — adjustment: +5%
   c. Two major competitors acquired in past 18 months (market consolidation)
      — strength: MODERATE — adjustment: +8%
   d. Revenue growth decelerated from 35% to 18% YoY (earnings report)
      — strength: MODERATE — adjustment: +5%
      (Slower-growth companies more likely to accept acquisition offers)

   Signals AGAINST (-):
   a. Founder still holds 25% voting control and has said "we're building for
      the long term" (recent interview)
      — strength: STRONG — adjustment: -12%
   b. Stock price at all-time high — acquirer must pay steep premium
      — strength: MODERATE — adjustment: -5%
   c. Current antitrust environment — FTC blocking more deals
      — strength: WEAK — adjustment: -3%

3. SYNTHESIS
   Starting probability (base rate):  6%
   Signals for:  +20% +5% +8% +5% =  +38%
   Signals against: -12% -5% -3%   =  -20%
   Net adjustment:                     +18%
   Raw probability:                    24%

   Sanity check: ~1 in 4 feels right given the strong M&A advisor signal
   balanced against founder control.
   Final probability: 25%

4. KEY ASSUMPTIONS
   - Goldman engagement is for M&A (not debt restructuring):
     If wrong, probability drops to 8%
   - Founder is willing to sell at the right price:
     If wrong (founder vetoes any deal), probability drops to 3%
   - Regulatory environment doesn't tighten further:
     If wrong, probability drops to 18%

5. RESOLUTION
   Date: January 31, 2026
   Criteria: Definitive merger agreement announced (not just rumors)
   Data source: SEC EDGAR (8-K filing), Bloomberg terminal
```

### Example 2: Technology Adoption

**Question**: "Will WebAssembly (Wasm) reach mainstream server-side adoption by 2027?"

```
PREDICTION: >20% of new cloud-deployed services will use Wasm runtimes by
end of 2027

1. REFERENCE CLASS (Outside View)
   Technology adoption lifecycle (Rogers curve):
   - Innovators (2.5%) → Early Adopters (13.5%) → Early Majority (34%)
   - Crossing from Early Adopters to Early Majority typically takes 3-5 years
     after first production deployments
   - First serious server-side Wasm deployments: ~2022 (Fermyon, Cosmonic)
   - Current status (2025): Late Early Adopter stage

   Historical analogues for infrastructure tech adoption:
   - Containers (Docker 2013 → mainstream 2017-2018): ~4-5 years
   - Kubernetes (2014 → mainstream 2018-2019): ~4-5 years
   - Serverless (Lambda 2014 → mainstream 2018-2020): ~4-6 years

   Base rate for "infrastructure tech reaching 20% adoption within 5 years
   of first production use": ~30%

   Starting probability: 30%

2. SPECIFIC EVIDENCE (Inside View)
   Signals FOR (+):
   a. WASI standard maturing — WASI Preview 2 shipped, component model
      stabilizing (W3C working group)
      — strength: STRONG — adjustment: +8%
   b. Major cloud providers offering Wasm runtimes (Fastly, Cloudflare Workers,
      Azure, AWS exploring)
      — strength: STRONG — adjustment: +10%
   c. Docker adding Wasm support natively (announced 2022, shipping)
      — strength: MODERATE — adjustment: +5%

   Signals AGAINST (-):
   a. Ecosystem still fragmented — multiple competing runtimes, toolchain gaps
      — strength: STRONG — adjustment: -10%
   b. Containers already "good enough" for most workloads — weak forcing
      function to switch
      — strength: STRONG — adjustment: -8%
   c. Wasm language support uneven — great for Rust/C++, mediocre for Python/JS
      — strength: MODERATE — adjustment: -5%

   Leading indicators to track:
   - CNCF survey: % of respondents evaluating/using Wasm
   - Job postings mentioning Wasm (Indeed/LinkedIn trend)
   - GitHub stars and contributors for top Wasm runtimes (wasmtime, wasmer)
   - WASI spec milestone dates vs planned dates

3. SYNTHESIS
   Starting probability (base rate):  30%
   Signals for:  +8% +10% +5%      =  +23%
   Signals against: -10% -8% -5%   =  -23%
   Net adjustment:                      0%
   Final probability: 30%

   Interpretation: The positive and negative signals roughly cancel out.
   The base rate from analogous infrastructure technologies holds.
   This is genuinely uncertain — the "chasm" crossing is the key risk.

4. KEY ASSUMPTIONS
   - "Mainstream" defined as >20% of NEW deployments (not total installed base)
   - WASI component model reaches 1.0 stable by mid-2026
     If delayed beyond 2026: probability drops to 15%
   - No competing paradigm emerges (e.g., eBPF expanding scope):
     If strong competitor: probability drops to 20%

5. RESOLUTION
   Date: December 31, 2027
   Criteria: CNCF annual survey shows >20% respondents using Wasm in production
   Data source: CNCF Annual Survey, Datadog Container Report
```

### Example 3: Geopolitical Forecast

**Question**: "Will US-China trade tensions escalate significantly in 2025?"
(Defined as: new tariffs >25% on >$100B of goods, or export controls expanded
to 3+ new technology categories)

```
PREDICTION: Significant escalation of US-China trade tensions in 2025

1. REFERENCE CLASS (Outside View)
   Historical trade conflict escalation pattern:
   - US-China trade relations since 2018: escalation occurred in 4 of 7 years
   - In election year +1 (new/returning administration): escalation rate ~60%
   - Trade wars historically escalate in steps, with retaliation cycles

   Starting probability: 55%

2. SPECIFIC EVIDENCE (Inside View)
   Signals FOR (+):
   a. Administration rhetoric on China hawkish across both parties
      — strength: STRONG — adjustment: +10%
   b. Semiconductor export controls already expanding (ASML, Tokyo Electron)
      — strength: STRONG — adjustment: +8%
   c. China retaliating with rare earth export restrictions
      — strength: MODERATE — adjustment: +5%

   Signals AGAINST (-):
   a. Business lobbying against further tariffs (Chamber of Commerce, farm lobby)
      — strength: MODERATE — adjustment: -5%
   b. Inflation concerns create political cost for tariffs
      — strength: MODERATE — adjustment: -5%
   c. Diplomatic channels active (recent bilateral meetings)
      — strength: WEAK — adjustment: -3%

   Scenario mapping:
   ┌─────────────────────────┬─────────────┬────────────────────┐
   │ Scenario                │ Probability │ Key trigger        │
   ├─────────────────────────┼─────────────┼────────────────────┤
   │ Major escalation        │ 25%         │ Taiwan crisis or   │
   │ (new tariffs + controls │             │ tech IP theft case │
   │  + retaliatory cycle)   │             │                    │
   ├─────────────────────────┼─────────────┼────────────────────┤
   │ Moderate escalation     │ 40%         │ Incremental tariff │
   │ (meets our threshold)   │             │ increases + 1-2    │
   │                         │             │ new export controls│
   ├─────────────────────────┼─────────────┼────────────────────┤
   │ Status quo / minor      │ 30%         │ Diplomatic deals,  │
   │ changes                 │             │ election distraction│
   ├─────────────────────────┼─────────────┼────────────────────┤
   │ De-escalation           │ 5%          │ Grand bargain      │
   │ (reduced tariffs)       │             │ (historically rare)│
   └─────────────────────────┴─────────────┴────────────────────┘

   P(meets our escalation threshold) = 25% + 40% = 65%

3. SYNTHESIS
   Starting probability (base rate):  55%
   Signals for:  +10% +8% +5%      =  +23%
   Signals against: -5% -5% -3%    =  -13%
   Net adjustment:                     +10%
   Raw probability:                    65%

   Cross-check with scenario mapping: 65% — consistent.
   Final probability: 65%

4. KEY ASSUMPTIONS
   - No major geopolitical crisis (Taiwan strait) that causes extreme
     escalation or extreme restraint: If crisis occurs, split to
     80% (escalation) or 20% (restraint/avoidance)
   - US economy remains stable: If recession hits, probability drops
     to 45% (political cost of tariffs rises)
   - China does not make major trade concessions preemptively:
     If it does, probability drops to 30%

5. RESOLUTION
   Date: December 31, 2025
   Criteria: Cumulative new tariffs >25% on >$100B goods OR export controls
   expanded to 3+ new technology categories (per USTR/BIS announcements)
   Data source: USTR tariff schedule, BIS Entity List updates, Congressional
   Research Service reports
```

---

## Fermi Estimation Techniques

Fermi estimation is the art of making reasonable order-of-magnitude guesses
by breaking unknowable questions into smaller, estimable pieces.

### Step-by-Step Process

```
1. DEFINE the quantity you want to estimate
   → Be specific about units, scope, and timeframe

2. DECOMPOSE into factors you can estimate independently
   → Prefer multiplication chains: A × B × C
   → Each factor should be something you can reason about

3. ESTIMATE each factor
   → Use round numbers (powers of 10 when possible)
   → State your confidence range for each factor

4. MULTIPLY and sanity-check
   → Does the result pass the "smell test"?
   → Cross-check with any known anchors

5. STATE your uncertainty
   → Fermi estimates are typically accurate within 1 order of magnitude
   → Give a range: [estimate / 3, estimate × 3] is a reasonable default
```

### Common Reference Anchors

Keep these memorized for quick estimation:

```
POPULATION
  World:        ~8 billion
  US:           ~340 million
  EU:           ~450 million
  China:        ~1.4 billion
  India:        ~1.4 billion

ECONOMICS
  World GDP:           ~$100 trillion
  US GDP:              ~$28 trillion
  US median household: ~$75,000/year
  US federal budget:   ~$6.5 trillion
  S&P 500 total cap:   ~$45 trillion

TIME
  Seconds in a day:    ~86,400 (~10^5)
  Seconds in a year:   ~31.5 million (~3 × 10^7)
  Working hours/year:  ~2,000

TECHNOLOGY
  Global internet users:    ~5.5 billion
  Global smartphone users:  ~4.5 billion
  AWS annual revenue:       ~$90 billion
  Global IT spending:       ~$5 trillion
  GitHub developers:        ~100 million

INDUSTRY SIZES (annual, global)
  Cloud computing:     ~$600 billion
  Semiconductor:       ~$600 billion
  Pharmaceutical:      ~$1.5 trillion
  Automotive:          ~$3 trillion
  Agriculture:         ~$3 trillion
  E-commerce:          ~$6 trillion
```

### Worked Fermi Examples

**Example A: Estimating the TAM for an AI code review tool**

```
Question: What is the annual TAM for an AI-powered code review SaaS?

Decomposition:
  TAM = (Number of professional developers)
      × (% who do code reviews regularly)
      × (willingness to pay for tooling)
      × (average annual price)

Estimates:
  Professional developers worldwide: ~30 million
  (GitHub has 100M accounts, but ~30% are professional, and
   not all professionals use GitHub)

  % who do code reviews: ~60%
  (Standard in companies > 50 engineers, less common in small shops)

  Target market (teams that would buy SaaS): ~40%
  (Enterprise and mid-market; small teams use free tools)

  Annual price per seat: ~$300/year
  (Comparable: GitHub Copilot ~$200, Snyk ~$400, middle ground)

Calculation:
  30M × 0.60 × 0.40 × $300 = $2.16 billion

Sanity check:
  - GitHub revenue ~$2B (broader product, ~4M paid users)
  - Snyk valued at $7B (code security, related space)
  - $2B TAM is plausible for a focused code review tool

Result: ~$2 billion TAM (range: $700M to $6B)
```

**Example B: Estimating daily active queries to a search engine**

```
Question: How many search queries does Google process per day?

Decomposition:
  Queries/day = (Internet users who use Google)
              × (searches per user per day)

Estimates:
  Global internet users: ~5.5 billion
  Google market share: ~90%
  Google users: 5.5B × 0.90 = ~5 billion
  But not all use it daily: ~50% daily active rate
  Daily active Google searchers: ~2.5 billion

  Searches per active user per day: ~3-4
  (Some people search 10+ times, many search once or not at all)

Calculation:
  2.5 billion × 3.5 = ~8.5 billion queries/day

Sanity check:
  Published figure (Google): ~8.5 billion searches/day (2024)
  Our estimate nailed it — sometimes Fermi estimation gets lucky.

Result: ~8.5 billion/day (range: 3B to 25B)
```

### Order of Magnitude Sanity Checks

After any estimate, verify it makes sense:

```
CHECK 1: Per-person reasonableness
  Divide by relevant population. Is the per-person number realistic?
  "$50B market ÷ 340M Americans = $147/person" — plausible?

CHECK 2: Comparison to known quantities
  Is your estimate bigger or smaller than things you know?
  "Our estimate of X is $3B — that's 5% of AWS revenue. Reasonable?"

CHECK 3: Growth rate implied
  If you're estimating a future state, what annual growth rate is implied?
  >50% sustained growth for >3 years is extremely rare.

CHECK 4: Upper bound test
  What is the theoretical maximum? Is your estimate within it?
  "Total possible customers × maximum price = ceiling"
```

---

## Prediction Market Patterns

### Interpreting Market Prices as Probabilities

Prediction market prices map to probabilities, but with important caveats:

```
Market price $0.65 for "Event X occurs"
  → Naive interpretation: 65% probability
  → Adjusted interpretation: depends on market quality

Adjustment factors:
  Liquid market (Polymarket, Metaculus with many forecasters):
    Price ≈ true probability (±3-5%)

  Thin market (<50 traders, <$10K volume):
    Price is noisy — treat as ±15% uncertainty
    A $0.65 price could represent 50-80% true probability

  Binary vs. multi-outcome:
    Binary markets are more reliable
    Multi-outcome markets often have probabilities summing to >100%
    (overround) — normalize before interpreting
```

### Common Prediction Market Biases

| Bias | Description | Impact | Correction |
|------|-------------|--------|------------|
| Favorite-longshot | Favorites underpriced, longshots overpriced | Longshot events appear ~2-3x more likely than they are | If market says 5%, true probability may be 2-3% |
| Recency | Recent events dominate pricing | Probability spikes after news, then slowly reverts | Wait 24-48h after major news before trusting market prices |
| Liquidity premium | Illiquid contracts trade at a discount | Prices biased toward 50% in thin markets | Weight liquid markets more heavily |
| Expiration clustering | Prices converge to 0 or 1 near expiration | Mid-probability contracts vanish near deadline | Most useful signal is months before resolution |
| Hedging distortion | Traders hedging other positions, not expressing beliefs | Prices reflect risk management, not pure probability | Cross-reference with non-market forecasts |

### Aggregation Methods

When combining multiple probability estimates (markets, experts, models):

```
SIMPLE AVERAGE
  P = (P1 + P2 + P3) / 3
  Use when: Sources are roughly equally credible
  Weakness: Susceptible to outliers

MEDIAN
  P = middle value of sorted estimates
  Use when: One source might be badly miscalibrated
  Weakness: Ignores magnitude of disagreement

TRIMMED MEAN
  Drop highest and lowest, average the rest
  P = average(P2 ... Pn-1) after sorting
  Use when: 5+ sources, want outlier robustness

EXTREMIZED AVERAGE
  P_avg = simple average
  P_extremized = P_avg^a / (P_avg^a + (1-P_avg)^a), where a > 1
  Typical a = 1.5 to 2.5 (more extremizing with more independent sources)
  Use when: Sources are genuinely independent (not reading each other)
  Rationale: If 5 independent sources all say 70%, the true probability
  is likely higher than 70% — shared info should push further from 50%

CONFIDENCE-WEIGHTED AVERAGE
  P = Σ(wi × Pi) / Σ(wi)
  where wi = track record score or source reliability
  Use when: Sources have known, differing track records
```

### When Markets Beat Experts (and Vice Versa)

```
MARKETS TEND TO WIN when:
  ✓ Large, liquid, diverse participant pool
  ✓ Question is well-defined with clear resolution criteria
  ✓ Information is widely distributed (no single expert has edge)
  ✓ Time horizon is 1 month to 2 years
  Examples: Election outcomes, product launch dates, economic indicators

EXPERTS TEND TO WIN when:
  ✓ Question requires deep domain-specific knowledge
  ✓ Market is thin or participants lack domain context
  ✓ Very long time horizons (>5 years) — markets discount distant futures
  ✓ Novel situations with no historical market precedent
  Examples: Technical feasibility, scientific breakthroughs, niche regulation

BEST PRACTICE: Use both
  Start with the market price, then adjust using expert insight.
  Treat the market as the prior and expert analysis as an update.
```

---

## Update Protocol

### Bayesian Updating Worked Example

```
SCENARIO: You predicted 30% chance that Company Z launches Product A in Q1.
New evidence: A leaked internal slide shows a Q1 launch timeline.

STEP 1: State the prior
  P(launch in Q1) = 0.30

STEP 2: Assess the evidence
  E = leaked slide showing Q1 timeline
  How likely is this evidence if the launch IS happening in Q1?
    P(E | launch) = 0.85
    (Internal slides usually reflect real plans, but plans change)
  How likely is this evidence if the launch is NOT in Q1?
    P(E | no launch) = 0.15
    (Could be outdated slide, aspirational, or decoy)

STEP 3: Calculate the likelihood ratio
  LR = P(E | launch) / P(E | no launch) = 0.85 / 0.15 = 5.67

STEP 4: Convert prior to odds, multiply, convert back
  Prior odds = 0.30 / 0.70 = 0.429
  Posterior odds = 0.429 × 5.67 = 2.43
  Posterior probability = 2.43 / (1 + 2.43) = 0.71

STEP 5: State the update
  Prior: 30% → Posterior: 71%
  Update magnitude: +41 percentage points
  This is a LARGE update, appropriate because the evidence (internal
  planning document) is strong and directly relevant.
```

### Evidence Strength Classification

How much to update based on different types of evidence:

```
EVIDENCE TIER 1 — Large update (likelihood ratio 5-20x)
  → Official announcement or regulatory filing
  → Confirmed internal document (not rumor)
  → Directly observed outcome of prerequisite event
  → Multiple independent strong sources confirming same fact
  Typical update: ±15-30 percentage points

EVIDENCE TIER 2 — Moderate update (likelihood ratio 2-5x)
  → Credible journalist report with named sources
  → Statistical data that changes the base rate
  → Expert with strong track record changing their view
  → Structural/policy change that alters incentives
  Typical update: ±5-15 percentage points

EVIDENCE TIER 3 — Small update (likelihood ratio 1.2-2x)
  → Rumor from semi-credible source
  → Anecdotal evidence (single data point)
  → Social media sentiment shift
  → Expert opinion without new information
  Typical update: ±2-5 percentage points

EVIDENCE TIER 4 — Negligible update (likelihood ratio ~1x)
  → Repetition of previously known information
  → Pundit opinion with no domain expertise
  → Vague statement open to multiple interpretations
  → Evidence equally consistent with both outcomes
  Typical update: ±0-2 percentage points (or skip entirely)
```

### When to Make Large vs. Small Updates

```
MAKE A LARGE UPDATE when:
  • Evidence directly addresses your key uncertainty
  • The source has a strong track record on this topic
  • The evidence would be very surprising if your prediction were correct
    (or very unsurprising if it were wrong)
  • Multiple independent signals shift in the same direction simultaneously

MAKE A SMALL UPDATE when:
  • Evidence is tangentially related to your prediction
  • The source's reliability is uncertain
  • The evidence is consistent with multiple interpretations
  • You've already incorporated similar evidence

RESIST UPDATING when:
  • The "evidence" is just someone restating the consensus
  • A vivid anecdote feels compelling but carries no statistical weight
  • You're reacting emotionally (fear, excitement) rather than analytically
  • The evidence source has an obvious incentive to mislead
```

### Common Updating Mistakes

| Mistake | Description | Fix |
|---------|-------------|-----|
| Over-updating on vivid events | A dramatic single event shifts your view by 20+ points when the base rate barely moved | Ask: "Does this event actually change the base rate, or just my emotional state?" |
| Under-updating on base rate changes | New data shows the reference class frequency shifted, but you keep your old anchor | Periodically re-derive the base rate from scratch instead of only adjusting incrementally |
| Asymmetric updating | Updating strongly on confirming evidence, weakly on disconfirming evidence | Force yourself to calculate the likelihood ratio for disconfirming evidence explicitly |
| Double-counting | Updating on a news article, then updating again on a tweet quoting the same article | Track the original source — if two signals share the same root cause, count once |
| Failure to update | Knowing the evidence should change your view but not bothering because your current number "feels right" | Set calendar reminders to review active predictions monthly with fresh evidence |
| Stampede updating | A prediction market spikes, causing you to rush your update to match | Market moves are data, not commands — assess independently, then compare |

---

## Prediction Tracking & Scoring

### Prediction Ledger Format
```json
{
  "id": "pred_001",
  "created": "2025-01-15",
  "prediction": "OpenAI will release GPT-5 before July 2025",
  "confidence": 0.65,
  "domain": "tech",
  "time_horizon": "2025-07-01",
  "reasoning_chain": "...",
  "key_signals": ["leaked roadmap", "compute scaling", "hiring patterns"],
  "status": "active|resolved|expired",
  "resolution": {
    "date": "2025-06-30",
    "outcome": true,
    "evidence": "Released June 15, 2025",
    "brier_score": 0.1225
  },
  "updates": [
    {"date": "2025-03-01", "new_confidence": 0.75, "reason": "New evidence: leaked demo"}
  ]
}
```

### Accuracy Report Template
```
ACCURACY DASHBOARD
==================
Total predictions:     N
Resolved predictions:  N (N correct, N incorrect, N partial)
Active predictions:    N
Expired (unresolvable):N

Overall accuracy:      X%
Brier score:           0.XX

Calibration:
  Predicted 90%+ → Actual: X% (N predictions)
  Predicted 70-89% → Actual: X% (N predictions)
  Predicted 50-69% → Actual: X% (N predictions)
  Predicted 30-49% → Actual: X% (N predictions)
  Predicted <30% → Actual: X% (N predictions)

Strengths: [domains/types where you perform well]
Weaknesses: [domains/types where you perform poorly]
```

---

## Cognitive Bias Checklist

Before finalizing any prediction, check for these biases:

1. **Anchoring**: Am I fixated on the first number I encountered?
   - Fix: Deliberately consider the base rate before looking at specific evidence

2. **Availability bias**: Am I overweighting recent or memorable events?
   - Fix: Check the actual frequency, not just what comes to mind

3. **Confirmation bias**: Am I only looking for evidence that supports my prediction?
   - Fix: Actively search for contradicting evidence (steel-man the opposite)

4. **Narrative bias**: Am I choosing a prediction because it makes a good story?
   - Fix: Boring predictions are often more accurate

5. **Overconfidence**: Am I too sure?
   - Fix: If you've never been wrong at this confidence level, you're probably overconfident

6. **Scope insensitivity**: Am I treating very different scales the same?
   - Fix: Be specific about magnitudes and timeframes

7. **Recency bias**: Am I extrapolating recent trends too far?
   - Fix: Check longer time horizons and mean reversion patterns

8. **Status quo bias**: Am I defaulting to "nothing will change"?
   - Fix: Consider structural changes that could break the status quo

### Contrarian Mode
When enabled, for each consensus prediction:
1. Identify what the consensus view is
2. Search for evidence the consensus is wrong
3. Consider: "What would have to be true for the opposite to happen?"
4. If credible contrarian evidence exists, include a contrarian prediction
5. Always label contrarian predictions clearly with the consensus for comparison
