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

### Porter's Five Forces

Analyze industry attractiveness:

1. **Threat of New Entrants**: Capital requirements, economies of scale, brand loyalty, access to distribution, regulatory barriers
2. **Bargaining Power of Suppliers**: Concentration, switching costs, differentiation, forward integration threat
3. **Bargaining Power of Buyers**: Concentration, switching costs, price sensitivity, backward integration threat
4. **Threat of Substitutes**: Performance trade-offs, switching costs, buyer propensity to substitute
5. **Competitive Rivalry**: Number of competitors, industry growth, fixed costs, differentiation, exit barriers

Rate each force: Low / Medium / High with supporting evidence.

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
