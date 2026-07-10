---
name: strategist-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI business strategy -- frameworks, market analysis, competitive intelligence, and strategic planning methodologies"
runtime: prompt_only
---

# Business Strategy Expert Knowledge

## Strategic Analysis Frameworks

### SWOT Analysis

Map internal and external factors:

| | Helpful | Harmful |
|---|---------|---------|
| **Internal** | Strengths | Weaknesses |
| **External** | Opportunities | Threats |

Best practices:
- Be specific: "Strong brand recognition in enterprise segment" not just "Good brand"
- Prioritize: Rank items by impact
- Cross-reference: Look for SO (strength-opportunity) and WT (weakness-threat) combinations
- Action-oriented: Every SWOT item should suggest a strategic response
- Time-bound: Note whether each factor is stable, strengthening, or weakening

**SWOT Cross-Impact Matrix** — The real value of SWOT is in the intersections:

| | Opportunities | Threats |
|---|---|---|
| **Strengths** | SO strategies: Use strengths to capture opportunities (offensive) | ST strategies: Use strengths to neutralize threats (defensive) |
| **Weaknesses** | WO strategies: Fix weaknesses to unlock opportunities (investment) | WT strategies: Minimize weaknesses exposed by threats (survival) |

Prioritize: SO strategies first (highest ROI), then ST (protect position), then WO (selective investment), last WT (only if existential).

### Porter's Five Forces

Analyze industry attractiveness:

1. **Threat of New Entrants**: Capital requirements, economies of scale, brand loyalty, access to distribution, regulatory barriers
2. **Bargaining Power of Suppliers**: Concentration, switching costs, differentiation, forward integration threat
3. **Bargaining Power of Buyers**: Concentration, switching costs, price sensitivity, backward integration threat
4. **Threat of Substitutes**: Performance trade-offs, switching costs, buyer propensity to substitute
5. **Competitive Rivalry**: Number of competitors, industry growth, fixed costs, differentiation, exit barriers

Rate each force: Low / Medium / High with supporting evidence.

**Dynamic Five Forces**: Forces change over time. For each force, note the **trend direction** (strengthening/stable/weakening) and the **trigger event** that could shift it. A force rated "Low" today with a strengthening trend deserves more attention than a stable "Medium" force.

### PESTEL Analysis

Macro-environmental scanning:

| Factor | Key Questions |
|--------|--------------|
| **Political** | Government stability? Trade policies? Regulation changes? |
| **Economic** | GDP growth? Interest rates? Inflation? Exchange rates? |
| **Social** | Demographics? Cultural trends? Consumer behavior shifts? |
| **Technological** | Innovation pace? R&D spending? Automation trends? |
| **Environmental** | Climate regulations? Sustainability demands? Resource scarcity? |
| **Legal** | Employment law? IP protection? Competition law? Data privacy? |

### Framework Integration Methodology

Individual frameworks are lenses. Strategic insight comes from combining them. Here is how to synthesize multiple frameworks into a unified analysis:

**The Integration Cascade** — Use frameworks in dependency order:

```
Step 1: PESTEL (macro context)
  → Identifies external forces shaping the industry
  → Output: Which macro factors matter most? What is changing?

Step 2: Porter's Five Forces (industry structure)
  → PESTEL outputs feed directly into Porter's forces
  → Example: "AI adoption accelerating" (PESTEL-Tech) → "Threat of new entrants rising" (Porter)
  → Output: How attractive is this industry? Where is structural power?

Step 3: SWOT (company positioning within industry)
  → Porter's outputs define the external O/T quadrants
  → Internal assessment (S/W) is company-specific
  → Output: Where does this company sit relative to industry forces?

Step 4: Strategic Options Generation
  → SWOT cross-impact matrix generates candidate strategies
  → Porter's forces identify which strategies are structurally viable
  → PESTEL trends determine timing and urgency
```

**Cross-Framework Contradiction Resolution:**
When frameworks disagree, do not average or ignore — investigate:
- PESTEL says favorable + Porter says unattractive → Macro tailwind but bad industry structure (e.g., restaurant industry: everyone eats, but margins are terrible)
- SWOT says strong + Porter says high rivalry → Company advantage may erode faster than expected
- Resolution: State both findings, explain the tension, and let the tension inform the recommendation (e.g., "Enter but with a differentiation strategy that exploits the macro trend while avoiding head-on competition")

**Synthesis Quality Checklist:**
- Does the conclusion follow logically from framework outputs, or did you skip to a preferred answer?
- Did you weight frameworks by relevance (PESTEL matters more for market entry; Porter matters more for competitive strategy)?
- Are the frameworks consistent? If not, is the inconsistency explained?
- Could someone reconstruct your reasoning by reading the framework outputs alone?

### Market Sizing (TAM-SAM-SOM)

**TAM** (Total Addressable Market): Total market demand for a product/service.
```
TAM = (Total potential customers) x (Annual revenue per customer)
```

**SAM** (Serviceable Addressable Market): TAM segment you can reach.
```
SAM = TAM x (% you can realistically serve given geography, channels, capability)
```

**SOM** (Serviceable Obtainable Market): SAM you can realistically capture.
```
SOM = SAM x (Expected market share %)
```

Methods:
- **Top-down**: Start with industry reports, narrow to your segment
- **Bottom-up**: Start with unit economics, multiply by reachable customers
- **Value theory**: How much value does the solution create? What % can you capture?

### Worked Example: Netflix vs Blockbuster (2007)

**SWOT Analysis for Netflix:**
| Category | Item | Evidence |
|----------|------|----------|
| Strength | Streaming technology | First-mover in online streaming; DVD-by-mail eliminated late fees |
| Strength | Recommendation engine | Personalized suggestions increased engagement 60% |
| Weakness | Limited content library | Dependent on studio licensing deals |
| Weakness | High content acquisition cost | Margins compressed by licensing fees |
| Opportunity | Broadband adoption | US broadband penetration growing 30% YoY |
| Opportunity | International expansion | Untapped markets in Europe and Asia |
| Threat | Studio-owned platforms | Studios could bypass Netflix and go direct-to-consumer |
| Threat | Piracy | Illegal streaming as free alternative |

**Porter's Five Forces for Video Streaming (2007):**
| Force | Rating | Rationale |
|-------|--------|-----------|
| New Entrants | 2/5 | High capital needed for content + tech infrastructure |
| Supplier Power | 4/5 | Studios control content; few alternatives |
| Buyer Power | 3/5 | Low switching cost but high engagement reduces churn |
| Substitutes | 2/5 | No equivalent convenience at the time |
| Rivalry | 3/5 | Blockbuster dominant but slow to innovate |

**Strategic Insight**: Netflix's technology moat + Blockbuster's organizational inertia = classic disruption pattern. Blockbuster's $6B revenue masked its vulnerability to a $1B challenger with superior unit economics. Confidence: **High (90%)** — outcome confirmed by Blockbuster's 2010 bankruptcy.

### Competitive Positioning

**Positioning Map**: Plot competitors on 2 key dimensions (e.g., price vs. quality, breadth vs. depth).

**Competitive Advantage Sources**:
- Cost leadership: Lower cost structure than competitors
- Differentiation: Unique value proposition
- Focus/Niche: Serve a narrow segment exceptionally well
- Network effects: Value increases with more users
- Switching costs: Expensive or difficult for customers to leave

---

## Strategic Planning Methodologies

### OKR Framework (Objectives and Key Results)

```
Objective: [What you want to achieve -- qualitative, inspiring]
  KR1: [Measurable outcome 1]
  KR2: [Measurable outcome 2]
  KR3: [Measurable outcome 3]
```

Rules:
- 3-5 objectives per period
- 2-5 key results per objective
- Key results must be measurable (not tasks)
- Score 0.0 to 1.0; target 0.7 average (stretch goals)

### Strategy Canvas (Blue Ocean)

Compare your offering vs competitors across key factors:
```
Factor          | Competitor A | Competitor B | Your Offering
Price           | High         | Medium       | Low
Quality         | High         | Medium       | High
Ease of Use     | Low          | Medium       | High
Features        | Many         | Few          | Moderate
Support         | Good         | Poor         | Excellent
```

Identify factors to:
- **Eliminate**: Remove factors the industry takes for granted
- **Reduce**: Lower factors below industry standard
- **Raise**: Increase factors above industry standard
- **Create**: Introduce factors the industry has never offered

### Decision Matrix

| Option | Criterion 1 (w:30%) | Criterion 2 (w:25%) | Criterion 3 (w:25%) | Criterion 4 (w:20%) | Weighted Score |
|--------|---------------------|---------------------|---------------------|---------------------|----------------|
| A      | 4                   | 3                   | 5                   | 2                   | 3.55           |
| B      | 3                   | 5                   | 3                   | 4                   | 3.70           |
| C      | 5                   | 2                   | 4                   | 3                   | 3.55           |

---

## Competitive Intelligence

### Information Sources

| Source Type | Examples | Reliability |
|------------|----------|-------------|
| Public filings | SEC filings, annual reports | High |
| Press releases | Company announcements | Medium-High |
| Job postings | LinkedIn, careers pages | Medium |
| Product pages | Websites, pricing pages | Medium |
| Review sites | G2, Capterra, Trustpilot | Medium |
| Social media | LinkedIn, Twitter, Reddit | Medium-Low |
| Industry reports | Gartner, Forrester, McKinsey | High |
| Patents | USPTO, Google Patents | High |
| News coverage | TechCrunch, Bloomberg | Medium |

### Competitor Tracking Template

```
Company: [Name]
Last Updated: YYYY-MM-DD

Product: [Core offering]
Pricing: [Model and price points]
Positioning: [How they describe themselves]
Target Market: [Who they sell to]
Key Differentiators: [What makes them unique]
Recent Moves: [Product launches, funding, hires, partnerships]
Strengths: [What they do well]
Weaknesses: [Where they fall short]
Estimated Revenue: [If available]
Employee Count: [Growth indicator]
```

---

## Report Templates

### Executive Brief Template
```markdown
# Strategic Brief: [Topic]
**Date**: YYYY-MM-DD | **Author**: Strategist Hand

## Situation
[2-3 sentences describing the current state]

## Key Findings
1. [Most important finding]
2. [Second finding]
3. [Third finding]

## Recommendation
[Clear, actionable recommendation with rationale]

## Next Steps
- [ ] [Action item 1] -- [Owner] -- [Due date]
- [ ] [Action item 2] -- [Owner] -- [Due date]

## Risk Factors
- [Key risk 1 and mitigation]
- [Key risk 2 and mitigation]
```

### Strategy Memo Template (SCR Format)
```markdown
# Strategy Memo: [Topic]

## Situation
[What is happening -- neutral facts]

## Complication
[Why this matters -- the challenge or opportunity]

## Resolution
[What we should do about it -- the recommendation]

## Evidence
[Supporting data and analysis]

## Implementation
[How to execute the recommendation]
```

---

## Worked Examples

### Example 1: B2B SaaS Market Entry into Japan

**Context**: A US-based B2B SaaS company (project management tool, $15M ARR, 200 employees) evaluating entry into the Japanese market.

**PESTEL Analysis — Japan B2B SaaS (2025):**

| Factor | Assessment | Impact | Score (1-5) |
|--------|-----------|--------|-------------|
| **Political** | Stable democracy; strong US-Japan trade relations; Digital Agency pushing government digitization | Positive | 4 |
| **Economic** | GDP $4.2T; weak yen (150 JPY/USD) makes USD-priced SaaS expensive; enterprise IT spend growing 4% YoY | Mixed | 3 |
| **Social** | Aging workforce accelerates automation need; consensus-driven decision making lengthens sales cycles (avg 6-9 months); strong preference for local-language support | Critical constraint | 2 |
| **Technological** | High internet penetration (93%); cloud adoption lagging US by 3-5 years but accelerating; 5G rollout complete in urban areas | Opportunity | 4 |
| **Environmental** | ESG reporting mandated for listed companies from 2023; sustainability-linked procurement gaining traction | Moderate opportunity | 3 |
| **Legal** | APPI (Act on Protection of Personal Information) requires data residency consideration; strict labor laws affect HR SaaS | Compliance cost | 2 |

**PESTEL Score**: 18/30 — Moderately favorable. Key risk: social/cultural factors demand significant localization investment.

**Porter's Five Forces — Japan Project Management SaaS:**

| Force | Rating | Evidence |
|-------|--------|---------|
| New Entrants | 2/5 | High localization cost ($500K-$1M); relationship-driven market favors incumbents |
| Supplier Power | 1/5 | Cloud infrastructure (AWS Tokyo, Azure Japan) is commodity; no supplier concentration |
| Buyer Power | 4/5 | Enterprise buyers demand customization; long procurement cycles give buyers leverage; RFP-driven purchasing |
| Substitutes | 3/5 | Excel/spreadsheet culture deeply entrenched; domestic tools (Backlog, Jooto) have cultural fit advantage |
| Rivalry | 4/5 | Asana, Monday.com, Notion already present; domestic players Backlog (Nulab) and Redmine have loyal bases |

**Go-to-Market Recommendation:**

```
Strategy: Partner-Led Entry (not direct sales)
Timeline: 18 months to first enterprise deal

Phase 1 (Months 1-6): Foundation
  - Hire Country Manager (must be bilingual Japanese national)
  - Full UI/UX localization (not just translation — date formats, name order, honorifics)
  - Achieve ISMAP certification (required for government/enterprise procurement)
  - Data residency: Deploy on AWS Tokyo region
  - Budget: $800K

Phase 2 (Months 4-12): Channel Development
  - Sign 2-3 SIer (System Integrator) partners: target NTT Data, Fujitsu, NEC
  - Japanese SIers control 60% of enterprise software purchasing decisions
  - Co-develop integration with domestic tools (kintone, Sansan, freee)
  - Budget: $600K (partner enablement + integration development)

Phase 3 (Months 8-18): Market Penetration
  - Target mid-market first (500-2000 employees) — faster decision cycles than enterprise
  - Launch at Japan IT Week (Spring/Autumn) and SaaS Industry Conference
  - Content marketing: Japanese-language case studies, webinars with local customers
  - Target: 20 paying customers, $500K ARR by month 18
  - Budget: $400K

Total Investment: $1.8M over 18 months
Break-even: Month 30 (projected)
```

**Decision**: Proceed with caution. The $4.2T economy and cloud adoption tailwind justify the investment, but only with proper localization and channel strategy. Direct sales without SIer partnerships has a historically high failure rate (>70% for foreign SaaS in Japan).

---

### Example 2: Competitive Response — Major Player Enters Your Niche

**Context**: You run a $5M ARR vertical SaaS for veterinary clinics (500 customers, 15% market share). Salesforce just announced "Salesforce for Veterinary" — a vertical solution built on their platform.

**Threat Assessment:**

| Dimension | Your Position | Salesforce | Gap |
|-----------|--------------|------------|-----|
| Brand recognition | Niche leader | Global enterprise brand | Large — but irrelevant in vet niche |
| Product depth | Purpose-built (8 years domain expertise) | Horizontal platform with vertical skin | Strong advantage |
| Price point | $200/mo per clinic | $500/mo estimated (Salesforce pricing) | 2.5x cheaper |
| Implementation time | 2 weeks | 3-6 months (typical SF implementation) | Strong advantage |
| Integration depth | Deep PMS/PIMS integration | API-based, requires middleware | Strong advantage |
| Sales motion | Direct + word-of-mouth | Enterprise sales team + SI partners | Different segments |
| Switching cost for your customers | Moderate (data migration + retraining) | High (Salesforce ecosystem lock-in) | Neutral |

**Strategic Response Framework:**

```
IMMEDIATE (Week 1-4): Defend the Base
  1. Customer communication campaign
     - CEO letter to all 500 customers: "Our commitment to veterinary"
     - Emphasize: purpose-built > horizontal platform
     - Announce product roadmap acceleration

  2. Lock in at-risk accounts
     - Identify top 50 accounts by revenue
     - Offer annual contract discounts (15-20% for 2-year commitment)
     - Schedule QBRs with all enterprise accounts within 30 days

  3. Competitive battle card
     - Create internal sales doc: feature-by-feature comparison
     - "Why vets choose us over Salesforce" — 5 key differentiators
     - Objection handling for "shouldn't we go with the safe choice?"

SHORT-TERM (Month 2-6): Deepen the Moat
  4. Accelerate domain-specific features
     - AI-powered treatment plan suggestions (Salesforce can't match this)
     - Telemedicine integration (vertical-specific)
     - Inventory management tied to treatment protocols

  5. Build switching costs
     - Launch data analytics dashboard (clinics depend on historical trends)
     - Introduce multi-location management (target growing chains)
     - API marketplace for vet-specific integrations (lab equipment, imaging)

  6. Community defense
     - Launch "Vet Tech Community" — user forum + knowledge base
     - Annual user conference (even virtual — creates tribal loyalty)
     - Customer advisory board (top 10 clinics = co-development partners)

MEDIUM-TERM (Month 6-18): Counterattack
  7. Move upmarket selectively
     - Enterprise tier for 10+ location chains ($500/mo — match SF pricing)
     - Offer white-glove migration from legacy systems
     - This is the segment Salesforce will target — contest it

  8. Geographic expansion
     - Salesforce announcement creates awareness of the category
     - Ride the wave: "Already purpose-built, already proven"
     - Target UK, Australia, Canada (English-speaking, similar vet market structure)
```

**Pricing Response Decision Matrix:**

| Option | Revenue Impact | Competitive Effect | Risk |
|--------|---------------|-------------------|------|
| No change | Neutral | Salesforce still 2.5x more expensive | Low — price isn't the battleground |
| Cut prices 20% | -$1M ARR | Signals weakness; Salesforce won't match | High |
| Add premium tier | +$500K potential | Compete at enterprise level; justify R&D | Medium |
| Usage-based addon | +$300K potential | Expand ARPU without base price war | Low |

**Recommendation**: Add premium tier + usage-based addons. Do NOT cut base prices. Salesforce's entry validates your market — use it to raise your valuation narrative ("Salesforce sees a $2B market opportunity in vet SaaS — we already own 15%").

**Confidence**: Medium-High (75%) — Historical pattern: when Salesforce enters verticals, purpose-built incumbents retain 80%+ of existing customers. Risk is in new customer acquisition where brand matters more.

---

### Example 3: Platform Sunset Decision — Migrate or Maintain Legacy Product

**Context**: A mid-stage startup ($20M ARR) runs two products: a legacy desktop app (60% of revenue, declining 10% YoY) and a modern cloud product (40% of revenue, growing 50% YoY). Should they sunset the desktop app?

**Decision Matrix:**

| Criterion (Weight) | Option A: Maintain Both | Option B: Sunset in 12mo | Option C: Sunset in 24mo |
|--------------------|------------------------|--------------------------|--------------------------|
| Revenue protection (30%) | 5 — No disruption | 2 — Lose 40% of legacy revenue | 4 — Gradual migration |
| Engineering efficiency (25%) | 1 — Two codebases drain resources | 5 — Full focus on cloud | 3 — Phased transition |
| Customer satisfaction (20%) | 3 — Legacy stagnates | 2 — Forced migration angers users | 4 — Supported migration path |
| Market positioning (15%) | 2 — Confused narrative | 5 — Clear cloud-first story | 4 — Transitional narrative |
| Financial risk (10%) | 3 — Slow bleed sustainable | 2 — Revenue cliff risk | 4 — Manageable decline |
| **Weighted Score** | **2.95** | **3.35** | **3.75** |

**Recommendation**: Option C — 24-month sunset with structured migration program.

```
Migration Program:
  Months 1-6:   Feature parity audit; build top 20 missing cloud features
  Months 7-12:  Migration incentive (20% discount for annual cloud commitment)
  Months 13-18: Desktop enters maintenance-only mode; no new features
  Months 19-24: End-of-life announcement; dedicated migration support team
  Month 24:     Desktop product sunsets; legacy support for 6 more months

Financial Model:
  Current state:     $12M desktop + $8M cloud = $20M ARR
  Month 12 (projected): $9M desktop + $14M cloud = $23M ARR
  Month 24 (projected): $2M desktop + $22M cloud = $24M ARR
  Month 30 (projected): $0 desktop + $26M cloud = $26M ARR

  Net ARR risk: ~$3M from non-migrating desktop customers
  Offset: Engineering savings of $1.5M/yr + faster cloud feature velocity
```

---

## Financial Analysis Frameworks

### Unit Economics

Core metrics every strategy should quantify:

```
CAC (Customer Acquisition Cost)
  = Total Sales & Marketing Spend / New Customers Acquired
  Example: $500K spend / 100 new customers = $5,000 CAC

LTV (Lifetime Value)
  = ARPU x Gross Margin % x (1 / Churn Rate)
  Example: $500/mo x 80% x (1 / 0.03) = $13,333 LTV

LTV:CAC Ratio
  Target: > 3:1 for healthy SaaS
  Example: $13,333 / $5,000 = 2.67:1 (below target — reduce CAC or increase retention)

CAC Payback Period
  = CAC / (ARPU x Gross Margin %)
  Example: $5,000 / ($500 x 0.80) = 12.5 months
  Target: < 18 months for SaaS
```

**Unit Economics Health Check:**

| Metric | Danger Zone | Acceptable | Excellent |
|--------|------------|------------|-----------|
| LTV:CAC | < 1:1 | 3:1 | > 5:1 |
| CAC Payback | > 24 months | 12-18 months | < 12 months |
| Gross Margin | < 60% | 70-80% | > 80% |
| Net Revenue Retention | < 90% | 100-110% | > 120% |
| Logo Churn (monthly) | > 5% | 2-3% | < 1% |

### Revenue Modeling

**SaaS Revenue Waterfall:**

```
Beginning ARR:                    $10,000,000
  + New Business:                 +$3,000,000  (new logos)
  + Expansion:                    +$1,500,000  (upsell/cross-sell)
  - Contraction:                    -$500,000  (downgrades)
  - Churn:                        -$1,200,000  (lost customers)
  = Ending ARR:                   $12,800,000

Net New ARR:         $2,800,000
Net Revenue Retention:   113%  = ($10M + $1.5M - $0.5M - $1.2M) / $10M
Gross Revenue Retention:  88%  = ($10M - $0.5M - $1.2M) / $10M
```

**MRR Growth Decomposition:**

```
MRR Growth Rate = New MRR + Expansion MRR - Churned MRR - Contraction MRR
                  ─────────────────────────────────────────────────────────
                                     Beginning MRR

Quick Ratio = (New MRR + Expansion MRR) / (Churned MRR + Contraction MRR)
Target: > 4 for high-growth SaaS
```

### Break-Even Analysis

```
Break-Even Revenue = Fixed Costs / Gross Margin %

Example:
  Fixed Costs (monthly): $200K (salaries, rent, tools)
  Gross Margin: 80%
  Break-Even Revenue = $200K / 0.80 = $250K/month = $3M ARR

Break-Even Customers = Break-Even Revenue / ARPU
  = $250K / $500 = 500 customers
```

**Scenario Table:**

| Scenario | Fixed Costs | Gross Margin | Break-Even ARR | Break-Even Customers |
|----------|------------|--------------|-----------------|---------------------|
| Lean | $150K/mo | 85% | $2.1M | 353 |
| Base | $200K/mo | 80% | $3.0M | 500 |
| Growth | $350K/mo | 75% | $5.6M | 933 |

### Project Evaluation — Simplified DCF

Use for evaluating strategic investments (new market entry, build vs buy, major feature investment):

```
NPV = Σ [Cash Flow_t / (1 + r)^t] - Initial Investment

Where:
  r = discount rate (typically 10-15% for startups, 8-10% for established companies)
  t = year (0, 1, 2, ... n)
```

**Worked Example — Should we build a mobile app?**

```
Initial Investment: $500K (development cost)
Discount Rate: 12%

Year  | Incremental Revenue | Incremental Cost | Net Cash Flow | PV Factor | Present Value
------|--------------------|--------------------|---------------|-----------|-------------
  0   | $0                 | $500,000           | -$500,000     | 1.000     | -$500,000
  1   | $200,000           | $80,000            | $120,000      | 0.893     | $107,143
  2   | $400,000           | $100,000           | $300,000      | 0.797     | $239,158
  3   | $600,000           | $120,000           | $480,000      | 0.712     | $341,655
  4   | $700,000           | $130,000           | $570,000      | 0.636     | $362,204

NPV = $550,160  → Positive NPV → Project is financially justified
Payback Period: ~2.3 years (cumulative cash flow turns positive in Year 3)
```

**Decision Rule:**
- NPV > 0 → Proceed (project creates value)
- NPV < 0 → Reject (project destroys value)
- Compare NPV across mutually exclusive options; pick highest

---

## Go-to-Market Strategy Patterns

### Growth Motion Selection

| Growth Motion | Best For | Sales Cycle | CAC | Key Metric |
|--------------|---------|-------------|-----|------------|
| **Product-Led Growth (PLG)** | Self-serve products; low price point (<$500/mo); individual users | Minutes to days | Low ($50-$500) | Activation rate, PQL conversion |
| **Sales-Led Growth** | Enterprise products; complex deployment; >$50K ACV | Weeks to months | High ($5K-$50K) | Pipeline velocity, win rate |
| **Community-Led Growth** | Developer tools; open-source; platform products | Varies | Very low ($10-$100) | Community size, contribution rate |
| **Partner-Led Growth** | Market entry; regulated industries; ecosystem products | Varies | Medium ($1K-$10K) | Partner-sourced revenue % |

**PLG Funnel:**
```
Visitor → Sign-up → Activated User → PQL → Paid Customer → Expanded Account
  100%     10%        40%             25%      15%            30%

Key levers:
  - Sign-up friction: Reduce form fields, add SSO
  - Time-to-value: Get user to "aha moment" in < 5 minutes
  - PQL definition: User hits usage threshold that correlates with purchase
  - Expansion trigger: Team features, usage limits, premium capabilities
```

**Sales-Led Funnel:**
```
Lead → MQL → SQL → Opportunity → Proposal → Closed Won
 100%   20%   50%     60%          70%        30%

Key levers:
  - Lead quality: ICP fit scoring
  - MQL→SQL handoff: Alignment between marketing and sales
  - Discovery: Deep pain identification
  - Champion building: Enable internal advocate
  - Procurement: Legal/security review preparation
```

### Pricing Strategy Frameworks

**Value-Based Pricing (recommended for most SaaS):**
```
1. Quantify customer value created
   Example: Your tool saves 10 hours/week per user
   Value = 10 hrs x $75/hr x 52 weeks = $39,000/year

2. Capture 10-20% of value created
   Price = $39,000 x 15% = $5,850/year = $487/month

3. Validate with willingness-to-pay research
   Van Westendorp Price Sensitivity Meter:
   - "At what price is this too expensive?" → $600/mo
   - "At what price is this a bargain?" → $200/mo
   - "At what price does it seem expensive but you'd still consider?" → $450/mo
   - "At what price does it seem too cheap to trust?" → $100/mo
   → Optimal price range: $200-$450/mo
```

**Pricing Tier Architecture:**

```
Tier Structure (Good-Better-Best):

| | Starter | Professional | Enterprise |
|---|---------|-------------|------------|
| Target | Individual/SMB | Mid-market team | Large organization |
| Price | $29/mo | $99/mo/user | Custom (>$500/mo) |
| Anchor role | Drive adoption | Revenue driver (~60% of revenue) | Margin driver |
| Features | Core functionality | Full platform | Custom + SLA + support |
| Support | Self-serve/email | Priority email + chat | Dedicated CSM + phone |
| Billing | Monthly/Annual | Annual preferred | Annual contract |

Design principles:
  - Middle tier should be the obvious best value
  - Top tier exists to make middle tier look reasonable (anchoring effect)
  - Feature gates should align with natural usage growth
  - Price metric should scale with value received (per user, per GB, per transaction)
```

**Competitive Pricing Analysis:**

```
Competitor Price Map:

Competitor    | Entry Price | Mid-Tier | Enterprise | Price Metric
-------------|-------------|----------|------------|-------------
Competitor A  | $49/mo      | $149/mo  | Custom     | Per user
Competitor B  | $0 (free)   | $99/mo   | $299/mo    | Flat rate
Competitor C  | $29/mo      | $79/mo   | Custom     | Per user
Your Product  | ???         | ???      | ???        | ???

Positioning options:
  - Price leader: 20-30% below average → requires cost advantage
  - Value leader: At or above average → requires clear differentiation
  - Premium: 30%+ above average → requires brand and feature superiority
```

### Channel Strategy

| Channel | Margin | Control | Scale | Best For |
|---------|--------|---------|-------|----------|
| Direct sales | High (85-95%) | Full | Slow | Enterprise, complex products |
| Inside sales | High (80-90%) | Full | Medium | Mid-market, $5K-$50K ACV |
| Self-serve | Highest (95%+) | Full | Fast | PLG, low ACV |
| Reseller/VAR | Low (60-70%) | Medium | Medium | Regional coverage, compliance |
| Marketplace (AWS/Azure) | Low (70-85%) | Low | Fast | Enterprise procurement shortcuts |
| System Integrator | Low (50-70%) | Low | Medium | Complex implementations |
| Affiliate/Referral | High (80-90%) | Low | Fast | Consumer, SMB |

### Launch Playbook Template

```
LAUNCH PLAYBOOK: [Product/Feature Name]
Launch Date: YYYY-MM-DD
Launch Type: [Major / Minor / Feature / Beta]

PRE-LAUNCH (T-8 weeks to T-0)
  Week -8: Finalize positioning and messaging
  Week -6: Create sales enablement materials (battle cards, one-pagers, demo script)
  Week -4: Brief analyst relations (Gartner, Forrester) if applicable
  Week -3: Seed beta customers (5-10 design partners); collect testimonials
  Week -2: Pre-brief press/media under embargo
  Week -1: Internal all-hands; sales team training; support team training

LAUNCH DAY (T-0)
  - Blog post (SEO-optimized)
  - Email to customer base
  - Social media campaign (LinkedIn, Twitter/X)
  - Press release (if major launch)
  - Product Hunt submission (if applicable)
  - In-app announcement for existing users
  - Founder/CEO LinkedIn post (highest engagement channel)

POST-LAUNCH (T+1 to T+8 weeks)
  Week +1: Monitor activation metrics; respond to all feedback
  Week +2: Publish customer case study
  Week +4: Webinar / live demo for pipeline
  Week +6: Analyze launch metrics vs targets
  Week +8: Retrospective and iteration plan

METRICS TO TRACK:
  - Awareness: Blog views, social impressions, press mentions
  - Activation: Sign-ups, trial starts, feature adoption rate
  - Revenue: Pipeline generated, deals influenced, new ARR
  - Sentiment: NPS from beta users, social sentiment, support ticket volume
```

---

## Scenario Planning

### Best / Base / Worst Case Framework

Structure every major strategic decision with three scenarios:

```
SCENARIO PLANNING: [Decision or Initiative]

                    | Worst Case      | Base Case       | Best Case
--------------------|-----------------|-----------------|------------------
Revenue impact      | [quantify]      | [quantify]      | [quantify]
Timeline            | [duration]      | [duration]      | [duration]
Key assumption      | [what goes wrong]| [most likely]   | [what goes right]
Probability         | [15-25%]        | [50-60%]        | [15-25%]
Trigger indicators  | [early signals] | [tracking metrics]| [early signals]
Response plan       | [pivot/exit]    | [continue/adjust]| [accelerate/expand]
```

**Worked Example — Launching a New Product Line:**

```
SCENARIO PLANNING: Launch enterprise analytics add-on ($200/mo)

                    | Worst Case (20%)  | Base Case (55%)   | Best Case (25%)
--------------------|-------------------|-------------------|-------------------
Adoption rate       | 5% of customers   | 15% of customers  | 30% of customers
Year 1 revenue      | $120K             | $360K             | $720K
Development cost    | $400K             | $400K             | $400K
Year 1 ROI          | -70%              | -10%              | +80%
Break-even          | Never (kill it)   | Month 18          | Month 8
Key assumption      | Customers don't   | Moderate demand;  | Strong demand;
                    | see value; churn  | gradual adoption  | pulls forward
                    | increases 2%      |                   | enterprise deals

Trigger Indicators:
  Worst: < 3% adoption after 3 months; NPS < 20 for add-on
  Base:  8-12% adoption after 3 months; positive but slow pipeline
  Best:  > 20% adoption after 3 months; inbound enterprise interest

Response Plans:
  Worst: Pivot to bundling analytics into existing plan (retention play)
  Base:  Continue; invest in onboarding and customer education
  Best:  Hire dedicated analytics PM; accelerate roadmap; raise prices 20%
```

**Expected Value Calculation:**

```
Expected Revenue = (Worst Revenue x Worst Prob) + (Base Revenue x Base Prob) + (Best Revenue x Best Prob)
                 = ($120K x 0.20) + ($360K x 0.55) + ($720K x 0.25)
                 = $24K + $198K + $180K
                 = $402K

Expected ROI = ($402K - $400K) / $400K = 0.5%
→ Marginal on expected value alone — proceed only if strategic upside justifies the bet
```

### Sensitivity Analysis

Identify which variables have the highest impact on outcomes:

```
SENSITIVITY ANALYSIS: New Market Entry

Base Case NPV: $550K

Variable            | -20% Change    | Base     | +20% Change    | Sensitivity
--------------------|---------------|----------|----------------|------------
Customer price      | $280K (-49%)  | $550K    | $820K (+49%)   | HIGH
Customer volume     | $310K (-44%)  | $550K    | $790K (+44%)   | HIGH
Churn rate          | $720K (+31%) | $550K    | $380K (-31%)   | HIGH
Development cost    | $650K (+18%) | $550K    | $450K (-18%)   | MEDIUM
CAC                 | $610K (+11%) | $550K    | $490K (-11%)   | MEDIUM
Discount rate       | $590K (+7%)  | $550K    | $510K (-7%)    | LOW
```

**Interpretation**: Price and volume are the highest-leverage variables. Strategy should prioritize pricing power and demand generation over cost optimization.

**Tornado Chart Format (text representation):**

```
Variable Impact on NPV (base = $550K):

Customer price     |████████████████████| -49% to +49%
Customer volume    |███████████████████ | -44% to +44%
Churn rate         |██████████████      | -31% to +31%
Development cost   |█████████           | -18% to +18%
CAC                |██████              | -11% to +11%
Discount rate      |████                | -7% to +7%
```

### Risk-Adjusted Decision Making

**Risk Register Template:**

| Risk | Probability (1-5) | Impact (1-5) | Risk Score | Mitigation | Residual Risk |
|------|-------------------|-------------|------------|------------|---------------|
| Key hire doesn't work out | 3 | 4 | 12 | Pipeline of 2 backup candidates | 6 |
| Competitor launches first | 4 | 3 | 12 | Focus on differentiation not speed | 8 |
| Technical architecture fails to scale | 2 | 5 | 10 | Prototype load test at 10x before commit | 4 |
| Regulatory change blocks approach | 1 | 5 | 5 | Legal review + pivot plan documented | 3 |
| Customer demand lower than projected | 3 | 4 | 12 | Pre-sell to 10 design partners before building | 6 |

**Risk-Adjusted NPV:**
```
Risk-Adjusted NPV = Base NPV x (1 - Risk Discount)

Where Risk Discount = Σ (Probability x Impact x Weight) for all material risks

Example:
  Base NPV: $550K
  Combined risk score: 0.15 (derived from risk register)
  Risk-Adjusted NPV: $550K x (1 - 0.15) = $467.5K
```

---

## Industry Analysis Templates

### Market Landscape Map

Plot all players in a market on two strategic dimensions:

```
MARKET LANDSCAPE: [Industry/Category]

                        Enterprise-Grade
                              |
                    Quadrant 2|  Quadrant 1
                    Niche     |  Market Leaders
                    Enterprise|
         Narrow ──────────────┼────────────── Broad
         Solution             |               Platform
                    Quadrant 3|  Quadrant 4
                    Point     |  Mass-Market
                    Solutions |  Platforms
                              |
                         SMB-Focused

Example — Project Management SaaS (2025):

Quadrant 1 (Leaders):   Asana, Monday.com, Smartsheet
Quadrant 2 (Niche):     Targetprocess (SAFe), Planview (PPM), Kantata (services)
Quadrant 3 (Point):     Todoist, Basecamp, Trello
Quadrant 4 (Platforms): Notion, ClickUp, Microsoft Planner

Your Position: [X]
Desired Position: [→ direction of strategic movement]
```

**Building a Landscape Map:**

1. Select two dimensions that represent the most important strategic trade-offs in the market
2. Commonly used axes:
   - Price / Complexity
   - Breadth of platform / Depth of solution
   - Enterprise / SMB focus
   - Horizontal / Vertical specialization
   - Self-serve / High-touch
3. Plot all known competitors (minimum 8-10 for useful map)
4. Identify white space — under-served quadrant combinations
5. Draw your strategic vector — where are you moving and why?

### Technology Adoption Lifecycle Positioning

```
THE ADOPTION CURVE:

  Innovators   Early        Early       Late        Laggards
  (2.5%)      Adopters     Majority    Majority     (16%)
              (13.5%)      (34%)       (34%)
     ___
    /   \
   /     \____
  /            \________
 /                      \_________
/                                  \___

       ↑                ↑
    THE CHASM      MAINSTREAM
    (biggest       (revenue
     risk point)    acceleration)
```

**Positioning by Stage:**

| Stage | Customer Profile | Sales Approach | Pricing Strategy | Key Risk |
|-------|-----------------|----------------|------------------|----------|
| Innovators | Tech enthusiasts; will tolerate bugs | Community; direct outreach | Free/very low; usage-based | Building for wrong use case |
| Early Adopters | Visionaries; want competitive advantage | Consultative selling; pilots | Value-based; ROI-justified | Chasm — can't cross to mainstream |
| Early Majority | Pragmatists; want proven solutions | References; case studies; demos | Competitive; published pricing | Scaling sales and support |
| Late Majority | Conservatives; want complete solutions | Standard procurement; RFPs | Bundled; enterprise agreements | Margin compression |
| Laggards | Skeptics; forced by circumstance | Compliance-driven; mandates | Legacy pricing; long contracts | Market is commoditizing |

**Chasm-Crossing Checklist:**
```
□ Whole product: Does the product solve the complete use case without workarounds?
□ References: Do you have 3-5 referenceable customers in the target segment?
□ Repeatability: Can you sell and implement without founder involvement?
□ Support: Can you support customers at scale (not just white-glove)?
□ Positioning: Is the messaging pragmatist-friendly (ROI, risk reduction) not visionary?
□ Competition: Have you defined the competitive set for pragmatist comparison?
□ Pricing: Is pricing simple, transparent, and aligned with buyer expectations?
```

### Value Chain Analysis

Decompose industry activities to find competitive advantage:

```
VALUE CHAIN: [Industry]

PRIMARY ACTIVITIES:
┌─────────────┬──────────────┬──────────────┬──────────────┬──────────────┐
│  Inbound    │  Operations  │  Outbound    │  Marketing   │  Service     │
│  Logistics  │              │  Logistics   │  & Sales     │              │
├─────────────┼──────────────┼──────────────┼──────────────┼──────────────┤
│ Sourcing    │ Production   │ Distribution │ Branding     │ Support      │
│ Inventory   │ Quality      │ Delivery     │ Pricing      │ Maintenance  │
│ Supplier    │ Assembly     │ Warehousing  │ Channel mgmt │ Returns      │
│ management  │ Testing      │ Order mgmt   │ Positioning  │ Training     │
└─────────────┴──────────────┴──────────────┴──────────────┴──────────────┘

SUPPORT ACTIVITIES:
┌──────────────────────────────────────────────────────────────────────────┐
│ Infrastructure: Finance, Legal, Management, Planning                     │
│ Human Resources: Recruiting, Training, Compensation, Culture             │
│ Technology: R&D, IT systems, Automation, Data analytics                  │
│ Procurement: Vendor selection, Negotiation, Contract management          │
└──────────────────────────────────────────────────────────────────────────┘
```

**Analysis Process:**

```
For each activity:
  1. Cost: What % of total cost does this activity represent?
  2. Value: How much does this activity contribute to customer willingness-to-pay?
  3. Capability: Rate your performance vs competitors (1-5)
  4. Strategic importance: Is this a source of differentiation? (Yes/No)

Activity             | Cost % | Value Contribution | Capability | Differentiator?
---------------------|--------|-------------------|------------|----------------
Inbound logistics    | 15%    | Low               | 3/5        | No
Operations           | 25%    | High              | 4/5        | Yes
Outbound logistics   | 10%    | Medium            | 3/5        | No
Marketing & Sales    | 30%    | High              | 2/5        | Needs improvement
Service              | 20%    | High              | 5/5        | Yes

Strategic Implications:
  - Invest: Operations (current strength + high value) and Service (strength to protect)
  - Improve: Marketing & Sales (high cost + low capability = drag on growth)
  - Optimize: Logistics (non-differentiating — minimize cost)
```

**SaaS-Specific Value Chain:**

```
┌────────────┬───────────────┬──────────────┬────────────────┬─────────────┐
│ Product    │ Customer      │ Customer     │ Customer       │ Expansion   │
│ Development│ Acquisition   │ Onboarding   │ Success        │ & Retention │
├────────────┼───────────────┼──────────────┼────────────────┼─────────────┤
│ R&D        │ Marketing     │ Implementation│ Support       │ Upsell      │
│ Design     │ Sales         │ Training     │ Account mgmt  │ Cross-sell  │
│ QA         │ Partnerships  │ Migration    │ Health scoring │ Renewals    │
│ Platform   │ Growth/PLG    │ Integration  │ Community      │ Advocacy    │
└────────────┴───────────────┴──────────────┴────────────────┴─────────────┘

Key insight for SaaS: The majority of LTV is created AFTER the initial sale.
Disproportionate investment should go to Onboarding → Success → Expansion.
```

---

## Strategic Analysis Anti-Patterns

Common cognitive traps that produce bad strategy. Actively check for these in every analysis:

| Anti-Pattern | Detection Question | Countermeasure |
|---|---|---|
| **Confirmation Bias** — Seeking data that supports pre-existing beliefs; ignoring contradictory evidence | "Did I search for disconfirming evidence with equal effort?" | For every key conclusion, explicitly search for the strongest counterargument |
| **Anchoring** — First number encountered dominates all later estimates (first source says "$10B market" and final estimate drifts toward $10B) | "Is my final estimate suspiciously close to the first number I found?" | Collect 3+ independent estimates; use both bottom-up and top-down methods; investigate any 2x+ divergence |
| **Strategy-by-Analogy** — "Uber did X, so we should do X in healthcare" without testing structural similarity | "What are the 3 most important differences between this situation and the analogy?" | Use analogies to generate hypotheses, never to validate conclusions |
| **Missing Causal Chain** — Clear start and desirable end, but no credible mechanism connecting them (Step 1 → ??? → Profit) | "What specifically happens between 'launch' and 'achieve outcome'?" | Every recommendation needs a testable causal chain: A → B → C → D |
| **Denominator Neglect** — Citing impressive absolutes while ignoring base rates ("10,000 users!" out of 2M impressions = 0.5%) | "Relative to what?" | Always present metrics as ratios/rates; compare to benchmarks |
| **Survivorship Bias** — Deriving strategy from winners only; ignoring that failed companies tried the same thing | "How many companies tried this and failed?" | Seek failure case studies; note success AND failure rates |
| **Planning Fallacy** — Timelines assuming everything goes right | "Does this plan require performing better than we ever have?" | Use reference class forecasting; add 30-50% buffer; present best/base/worst timelines |

---

## Uncertainty Quantification

### Expressing Uncertainty

**For quantitative estimates (market size, revenue, costs):**
- Never give a single number. Always give a range: "Market size: $8-12B (base estimate $10B)"
- State the confidence interval: "80% confident the market is between $8B and $12B"
- Identify the key variable driving the range: "Range is driven primarily by uncertainty in adoption rate (15-25%)"

**For qualitative assessments:**
- Use the calibrated confidence scale consistently:
  - **Very High (>90%)**: Would be genuinely surprised if wrong. Multiple high-quality sources agree.
  - **High (70-90%)**: Strong evidence, but plausible alternative interpretations exist.
  - **Medium (50-70%)**: Balanced evidence. Reasonable people could disagree.
  - **Low (30-50%)**: More uncertain than certain. Treat as hypothesis, not finding.
  - **Very Low (<30%)**: Speculative. Useful for scenario planning but not for action.

### Assumption Tracking

Every analysis rests on assumptions. Make them explicit:

```
ASSUMPTION REGISTER:

| # | Assumption | Confidence | Impact if Wrong | Validation Method |
|---|-----------|------------|-----------------|-------------------|
| 1 | Market grows 15% YoY | High | Changes TAM by +/- 30% | Track quarterly industry reports |
| 2 | No new regulation in 12mo | Medium | Could block market entry | Monitor regulatory pipeline |
| 3 | Key hire joins by Q2 | Medium | Delays launch 3-6 months | Pipeline status check monthly |
| 4 | Competitor does not cut price | Low | Margin compression 10-15% | Track competitor pricing weekly |
```

Flag any assumption rated "Low" that has "High" impact — these are the **strategic landmines** that deserve contingency plans.

### When to Say "We Don't Know"

It is better to say "insufficient data to assess" than to fabricate a confident-sounding answer. Specifically:
- If fewer than 2 independent sources support a data point, flag it as unverified
- If the key variable has a range wider than 3x (e.g., market could be $5B or $15B), call out that the analysis is highly sensitive to this input
- If you are extrapolating a trend beyond the data range, state the extrapolation explicitly

---

## Industry-Specific Strategic Patterns

Certain strategic dynamics recur within industry categories. Recognizing these patterns accelerates analysis:

### Platform / Marketplace Businesses
- **Winner-take-most dynamics**: Network effects create power-law outcomes. Market share of #1 player often exceeds #2 + #3 combined.
- **Chicken-and-egg problem**: Must solve supply and demand simultaneously. Common solutions: single-player mode, subsidize one side, constrain geography first.
- **Multi-homing risk**: If users can easily use multiple platforms, network effects weaken. Strategy must increase switching costs or exclusive value.
- **Key metric**: Liquidity (match rate between supply and demand). Revenue follows liquidity, not the reverse.

### B2B SaaS
- **Land-and-expand**: Initial deal size matters less than expansion potential. Net revenue retention >120% can drive growth even at 0 new logos.
- **Switching cost lifecycle**: Switching costs increase with integration depth, data accumulation, and workflow embedding. Year 1 churn is always highest.
- **Category creation vs. category entry**: Creating a new category requires 3-5x more marketing spend but yields pricing power. Entering an existing category is cheaper but forces competitive positioning.
- **Key metric**: Net Revenue Retention (NRR). Above 130% = exceptional. Below 100% = leaky bucket that marketing cannot fill.

### Consumer / D2C
- **Acquisition cost spiral**: As easy-to-reach audiences saturate, CAC rises. Growth requires channel diversification or organic/viral mechanics.
- **Brand as moat**: In commoditized categories, brand is the primary differentiation. Brand building requires consistency over years, not campaigns over months.
- **Retention curve shape**: If the retention curve flattens (users who stay past day 30 tend to stay indefinitely), invest in onboarding. If it keeps declining, the product has a retention problem, not an acquisition problem.
- **Key metric**: Cohort retention at day 30/60/90. Payback period on CAC.

### Regulated Industries (Healthcare, Finance, Insurance)
- **Compliance as moat**: Regulatory requirements (HIPAA, SOC2, PCI-DSS) are expensive to achieve but create durable barriers to entry.
- **Sales cycle reality**: Enterprise sales cycles of 6-18 months are normal. Budget accordingly. Premature scaling of sales teams is the #1 killer.
- **Build vs. partner**: In heavily regulated industries, partnering with incumbents (who have regulatory relationships) often beats trying to disrupt them directly.
- **Key metric**: Sales cycle length, regulatory approval timeline, compliance cost as % of revenue.
