---
name: lead-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI lead generation — web research, enrichment, scoring, deduplication, and report generation"
runtime: prompt_only
---

# Lead Generation Expert Knowledge

## Ideal Customer Profile (ICP) Construction

A good ICP answers these questions:
1. **Industry**: What vertical does your ideal customer operate in?
2. **Company size**: How many employees? What revenue range?
3. **Geography**: Where are they located?
4. **Technology**: What tech stack do they use?
5. **Budget signals**: Are they funded? Growing? Hiring?
6. **Decision-maker**: Who has buying authority? (title, seniority)
7. **Pain points**: What problems does your product solve for them?

### Company Size Categories
| Category | Employees | Typical Budget | Sales Cycle |
|----------|-----------|---------------|-------------|
| Startup | 1-50 | $1K-$25K/yr | 1-4 weeks |
| SMB | 50-500 | $25K-$250K/yr | 1-3 months |
| Enterprise | 500+ | $250K+/yr | 3-12 months |

### ICP Refinement Loop

The ICP should not be static. After every 3 report cycles, refine it:

1. **Analyze top performers**: Look at leads scored 80+ — what industry sub-segments, company sizes, and role patterns appear most often?
2. **Analyze low performers**: Look at leads scored below 40 — which ICP criteria were they missing? Were there false positives from overly broad keywords?
3. **Tighten criteria**: Narrow industry keywords (e.g., "fintech" becomes "payment infrastructure fintech"), adjust company size range, add or remove geographic regions, refine role titles.
4. **Track revisions**: Log each ICP revision with date, changes made, and rationale. This creates an audit trail showing how targeting improved over time.
5. **Measure impact**: Compare average lead score before and after each ICP revision. A well-refined ICP should produce higher average scores with fewer total leads — quality over quantity.

---

## Web Research Techniques for Lead Discovery

### Search Query Patterns
```
# Find companies in a vertical
"[industry] companies" site:crunchbase.com
"top [industry] startups [year]"
"[industry] companies [city/region]"

# Find decision-makers
"[title]" "[company]" site:linkedin.com
"[company] team" OR "[company] about us" OR "[company] leadership"

# Growth signals (high-intent leads)
"[company] hiring [role]" — indicates budget and growth
"[company] series [A/B/C]" — recently funded
"[company] expansion" OR "[company] new office"
"[company] product launch [year]"

# Technology signals
"[company] uses [technology]" OR "[company] built with [technology]"
site:stackshare.io "[company]"
site:builtwith.com "[company]"
```

### Source Quality Ranking
1. **Company website** (About/Team pages) — most reliable for personnel
2. **Crunchbase** — funding, company details, leadership
3. **LinkedIn** (public profiles) — titles, tenure, connections
4. **Press releases** — announcements, partnerships, funding
5. **Job boards** — hiring signals, tech stack requirements
6. **Industry directories** — comprehensive company lists
7. **News articles** — recent activity, reputation
8. **Social media** — engagement, company culture

### Industry-Specific Search Patterns

#### SaaS / Technology
```
# Company directories
site:g2.com/products "[category]"
site:capterra.com "[category] software"
site:producthunt.com "[product type]" "[year]"
"[category] software" site:crunchbase.com/organization

# Tech stack signals
site:stackshare.io "[technology]" decisions
site:builtwith.com/websites/[technology]

# Growth signals
"[company] SOC 2" OR "[company] ISO 27001"        — enterprise readiness
"[company] API" OR "[company] integration"          — platform maturity
"[company] case study" OR "[company] customer story" — traction evidence
```

#### Healthcare
```
# Directories & registries
site:healthcareittoday.com "[company]"
"digital health companies" site:crunchbase.com
"health tech" "[city/state]" site:angellist.co
"HIPAA compliant" "[category] software"

# Regulatory signals
"[company] FDA clearance" OR "[company] 510(k)"
"[company] HIPAA" OR "[company] HITRUST"
"[company] clinical trial" site:clinicaltrials.gov
```

#### Financial Services
```
# Directories & databases
site:fintechmagazine.com "top" "[category]"
"fintech companies" "[region]" site:crunchbase.com
"banking technology" OR "insurtech" site:cbinsights.com

# Compliance signals
"[company] SOX compliance" OR "[company] PCI DSS"
"[company] banking license" OR "[company] money transmitter"
"[company] Series [A/B/C]" "fintech"
```

#### E-commerce
```
# Directories & tools
site:apps.shopify.com "[category]"
site:store.bigcommerce.com "[category]"
"ecommerce brands" "[niche]" site:2pm.com OR site:modernretail.co

# Revenue signals
"[company] GMV" OR "[company] ARR"
"[company] warehouse" OR "[company] fulfillment center"
"[brand] DTC" OR "[brand] direct to consumer"
```

#### Manufacturing
```
# Directories
site:thomasnet.com "[product category]"
"manufacturing companies" "[city/state]" site:mfg.com
"industrial [category]" site:dnb.com

# Modernization signals
"[company] Industry 4.0" OR "[company] smart factory"
"[company] ERP" OR "[company] digital transformation"
"[company] ISO 9001" OR "[company] ISO 14001"
```

#### Industry Source Quick Reference
| Vertical | Primary Directories | Key Signal Keywords |
|----------|-------------------|---------------------|
| SaaS/Tech | G2, Capterra, ProductHunt, Crunchbase | "API launch", "SOC 2", "Series X" |
| Healthcare | HealthcareIT, ClinicalTrials.gov | "HIPAA", "FDA", "clinical trial" |
| Financial Services | CBInsights, Crunchbase | "PCI DSS", "banking license", "Series X" |
| E-commerce | Shopify App Store, ModernRetail | "GMV", "DTC", "fulfillment" |
| Manufacturing | ThomasNet, MFG.com | "Industry 4.0", "ISO 9001", "ERP" |

---

## Lead Enrichment Patterns

### Basic Enrichment (always available)
- Full name (first + last)
- Job title
- Company name
- Company website URL

### Standard Enrichment
- Company employee count (from About page, Crunchbase, or LinkedIn)
- Company industry classification
- Company founding year
- Technology stack (from job postings, StackShare, BuiltWith)
- Social profiles (LinkedIn URL, Twitter handle)
- Company description (from meta tags or About page)

### Deep Enrichment
- Recent funding rounds (amount, investors, date)
- Recent news mentions (last 90 days)
- Key competitors
- Estimated revenue range
- Recent job postings (growth signals)
- Company blog/content activity (engagement level)
- Executive team changes

### Enrichment Depth Escalation Strategy

Not all leads deserve the same enrichment investment. Use a two-pass approach:

1. **First pass (Standard depth)**: Enrich all discovered leads at Standard depth. This is cost-effective and provides enough data for initial scoring.
2. **Score checkpoint**: After the first pass, score all leads. Any lead scoring 70+ at Standard depth is a strong candidate.
3. **Second pass (Deep depth)**: Re-enrich only leads scoring 70+ at Deep depth. This focuses expensive research (funding history, news, competitive analysis) on leads most likely to convert.
4. **Skip threshold**: Leads scoring below 30 after Standard enrichment should not be enriched further — the data is unlikely to improve their score enough to matter.

This approach typically reduces total enrichment cost by 40-60% while maintaining the same output quality for top-tier leads.

### Email Pattern Discovery
Common corporate email formats (try in order):
1. `firstname@company.com` (most common for small companies)
2. `firstname.lastname@company.com` (most common for larger companies)
3. `first_initial+lastname@company.com` (e.g., jsmith@)
4. `firstname+last_initial@company.com` (e.g., johns@)

Note: NEVER send unsolicited emails. Email patterns are for reference only.

---

## Lead Scoring Framework

### Scoring Rubric (0-100)
```
ICP Match (30 points max):
  Industry match:     +10
  Company size match: +5
  Geography match:    +5
  Role/title match:   +10

Growth Signals (20 points max):
  Recent funding:     +8
  Actively hiring:    +6
  Product launch:     +3
  Press coverage:     +3

Enrichment Quality (20 points max):
  Email found:        +5
  LinkedIn found:     +5
  Full company data:  +5
  Tech stack known:   +5

Recency (15 points max):
  Active this month:  +15
  Active this quarter:+10
  Active this year:   +5
  No recent activity: +0

Accessibility (15 points max):
  Direct contact:     +15
  Company contact:    +10
  Social only:        +5
  No contact info:    +0
```

### Score Interpretation
| Score | Grade | Action |
|-------|-------|--------|
| 80-100 | A | Hot lead — prioritize outreach |
| 60-79 | B | Warm lead — nurture |
| 40-59 | C | Cool lead — enrich further |
| 0-39 | D | Cold lead — deprioritize |

---

## Lead Qualification Frameworks

### BANT Framework

Use BANT to quickly qualify leads during or after enrichment. Each dimension maps to data you can discover through web research.

| Dimension | Question | Research Signals |
|-----------|----------|-----------------|
| **Budget** | Can they afford the solution? | Funding rounds, revenue estimates, job postings for related roles, pricing tier of current tools |
| **Authority** | Is this person a decision-maker? | Title seniority (VP+, C-level, Director), reports to CEO/CTO, listed on "Leadership" page |
| **Need** | Do they have the problem you solve? | Job postings mentioning the pain point, tech stack gaps, competitor tool usage, complaints on forums |
| **Timeline** | Is there urgency to buy? | Contract renewals, compliance deadlines, product launches, recent leadership changes |

#### BANT Scoring Overlay
Apply these modifiers on top of the base lead score:
```
Budget confirmed (funding, revenue signal):   +5
Authority confirmed (VP+ or C-level):         +5
Need confirmed (pain point evidence):         +5
Timeline confirmed (urgency signal):          +5
                                     Max bonus: +20
```

### MEDDIC Framework

Use MEDDIC for complex / enterprise sales qualification where longer deal cycles demand deeper research.

| Dimension | Definition | What to Look For |
|-----------|-----------|-----------------|
| **Metrics** | Quantifiable outcomes the buyer cares about | Case studies they publish, KPIs in job postings, analyst reports, earnings calls |
| **Economic Buyer** | Person with budget authority to sign | CFO, CEO, VP Finance, or "Head of Procurement" listed on team pages |
| **Decision Criteria** | Factors they use to evaluate vendors | RFP documents, vendor comparison blog posts, compliance requirements, review site feedback |
| **Decision Process** | Steps from evaluation to purchase | Procurement team presence, legal/compliance review cycles, pilot program mentions |
| **Identify Pain** | Specific problems driving the purchase | Support forums, Glassdoor reviews, social media complaints, analyst reports on industry challenges |
| **Champion** | Internal advocate for your solution | Conference speakers, blog authors, open-source contributors, people who engage with your content |

#### MEDDIC Research Checklist
```
For each enterprise lead, attempt to discover:
[ ] At least one quantifiable metric they care about
[ ] The economic buyer's name and title
[ ] 2+ decision criteria (compliance, performance, price, integration)
[ ] Whether they run formal procurement (RFP, committee)
[ ] 1+ specific pain point with evidence
[ ] A potential internal champion (engaged user, tech advocate)
```

### Choosing Between BANT and MEDDIC

The `qualification_framework` setting controls which framework is applied. When set to "auto", use this decision table:

| Scenario | Recommended Framework |
|----------|----------------------|
| SMB / startup targets, short sales cycle | BANT |
| Enterprise targets, $100K+ deal size | MEDDIC |
| Mixed list with varied company sizes | BANT first pass, MEDDIC for A-grade enterprise leads |
| Time-constrained research | BANT (faster to assess) |

---

## Deduplication Strategies

### Matching Algorithm
1. **Exact match**: Normalize company name (lowercase, strip Inc/LLC/Ltd) + person name
2. **Fuzzy match**: Levenshtein distance < 2 on company name + same person
3. **Domain match**: Same company website domain = same company
4. **Cross-source merge**: Same person at same company from different sources → merge enrichment data

### Normalization Rules
```
Company name:
  - Strip legal suffixes: Inc, LLC, Ltd, Corp, Co, GmbH, AG, SA
  - Lowercase
  - Remove "The" prefix
  - Collapse whitespace

Person name:
  - Lowercase
  - Remove middle names/initials
  - Handle "Bob" = "Robert", "Mike" = "Michael" (common nicknames)
```

---

## Output Format Templates

### CSV Format
```csv
Name,Title,Company,Company URL,LinkedIn,Industry,Size,Score,Discovered,Notes
"Jane Smith","VP Engineering","Acme Corp","https://acme.com","https://linkedin.com/in/janesmith","SaaS","SMB (120 employees)",85,"2025-01-15","Series B funded, hiring 5 engineers"
```

### JSON Format
```json
[
  {
    "name": "Jane Smith",
    "title": "VP Engineering",
    "company": "Acme Corp",
    "company_url": "https://acme.com",
    "linkedin": "https://linkedin.com/in/janesmith",
    "industry": "SaaS",
    "company_size": "SMB",
    "employee_count": 120,
    "score": 85,
    "discovered": "2025-01-15",
    "enrichment": {
      "funding": "Series B, $15M",
      "hiring": true,
      "tech_stack": ["React", "Python", "AWS"],
      "recent_news": "Launched enterprise plan Q4 2024"
    },
    "notes": "Strong ICP match, actively growing"
  }
]
```

### Markdown Table Format
```markdown
| # | Name | Title | Company | Score | Grade | Qualification | Key Signal |
|---|------|-------|---------|-------|-------|---------------|------------|
| 1 | Jane Smith | VP Engineering | Acme Corp | 85 | A | BANT 4/4 | Series B funded, hiring |
| 2 | John Doe | CTO | Beta Inc | 72 | B | BANT 3/4 | Product launch Q1 2025 |
```

### CRM Export Field Mappings

When `crm_export_format` is configured, produce an additional file with CRM-native field names:

**HubSpot** (JSON):
| Lead Field | HubSpot Property |
|------------|-----------------|
| first_name | `firstname` |
| last_name | `lastname` |
| title | `jobtitle` |
| company | `company` |
| company_url | `website` |
| industry | `industry` |
| score | `hs_lead_status` (mapped: 80+ = "New", 60-79 = "Open", <60 = "In Progress") |

**Salesforce** (CSV):
| Lead Field | Salesforce Field |
|------------|-----------------|
| first_name | `FirstName` |
| last_name | `LastName` |
| title | `Title` |
| company | `Company` |
| company_url | `Website` |
| industry | `Industry` |
| score | `Rating` (mapped: 80+ = "Hot", 60-79 = "Warm", <60 = "Cold") |
| lead_source | `LeadSource` |

**Pipedrive** (JSON):
| Lead Field | Pipedrive Field |
|------------|----------------|
| full_name | `name` |
| title | `job_title` |
| company | `org_name` |
| company_url | `org_address` |
| notes | `note` |

---

## Worked Examples

### Example 1: Fintech SaaS Series A/B Companies (50-200 Employees)

**Objective**: Find 10 SaaS companies in the fintech space with 50-200 employees that recently raised Series A or B.

#### Step 1 — Define ICP
```
Industry:       Fintech / Financial Technology
Company size:   50-200 employees (SMB)
Funding stage:  Series A or Series B (raised within last 18 months)
Geography:      United States (primary), UK/EU (secondary)
Decision-maker: VP Engineering, CTO, or Head of Product
Pain points:    Scaling infrastructure, compliance automation, developer tooling
```

#### Step 2 — Execute Search Queries
```
# Primary discovery queries
"fintech" "series A" OR "series B" site:crunchbase.com/organization
"fintech startup" "raised" "$" "2025" OR "2024" site:techcrunch.com
site:news.crunchbase.com "fintech" "series A" OR "series B"

# Employee count validation
"fintech" "50" OR "100" OR "150" "employees" site:linkedin.com/company
site:builtin.com/companies/fintech "51-200 employees"

# Growth signals
"fintech" hiring "senior engineer" OR "staff engineer" site:linkedin.com/jobs
"fintech startup" "SOC 2" OR "PCI DSS" — compliance-ready = selling to banks
```

#### Step 3 — Enrich and Score Each Lead
```
For each discovered company, gather:
  1. Company website → About page → leadership team, employee count
  2. Crunchbase profile → funding amount, date, investors, total raised
  3. LinkedIn company page → exact employee count, recent hires
  4. Job boards → open roles (signals growth and tech stack)
  5. Press releases → product launches, partnerships, customer wins

Scoring example for "PayFlow Inc":
  ICP Match:         25/30 (fintech ✓, 130 employees ✓, US ✓, CTO found ✓, no geography bonus)
  Growth Signals:    18/20 (Series B $18M ✓, hiring 8 engineers ✓, product launch ✓)
  Enrichment:        15/20 (LinkedIn ✓, full company data ✓, tech stack ✓, no direct email)
  Recency:           15/15 (funding announced 3 weeks ago)
  Accessibility:     10/15 (company contact form, CTO LinkedIn)
  TOTAL:             83/100 → Grade A
```

#### Step 4 — Final Output (top 3 of 10)
| # | Name | Title | Company | Employees | Funding | Score | Key Signal |
|---|------|-------|---------|-----------|---------|-------|------------|
| 1 | Sarah Chen | CTO | PayFlow Inc | 130 | Series B, $18M | 83 | Funded 3 weeks ago, hiring 8 engineers |
| 2 | Marcus Rivera | VP Engineering | LendStack | 85 | Series A, $12M | 78 | Launched API platform Q4, SOC 2 certified |
| 3 | Priya Patel | Head of Product | ComplianceAI | 62 | Series A, $8M | 75 | Hiring product + eng, regulatory focus |

---

### Example 2: Enterprise AI/ML Decision-Makers

**Objective**: Identify decision-makers at enterprise companies (500+ employees) that are actively adopting AI/ML tools.

#### Step 1 — Define ICP
```
Industry:       Any (cross-industry AI adoption)
Company size:   500+ employees (Enterprise)
Signals:        Active AI/ML adoption (hiring, projects, tool procurement)
Geography:      North America
Decision-maker: VP/Director of Data Science, Head of AI/ML, CTO, Chief Data Officer
Pain points:    ML model deployment, data pipeline scaling, AI governance
```

#### Step 2 — Execute Search Queries
```
# Identify companies investing in AI
"head of AI" OR "VP data science" OR "chief data officer" hiring site:linkedin.com
"[company] machine learning" "team" OR "department" site:linkedin.com/company
"AI adoption" OR "ML platform" "enterprise" site:venturebeat.com OR site:techcrunch.com

# Conference and community signals
"speaker" "machine learning" OR "AI" site:neurips.cc OR site:icml.cc
"[company] MLOps" OR "[company] AI infrastructure" site:github.com

# Budget and procurement signals
"AI budget" OR "ML tools" RFP site:gov OR site:rfpdb.com
"[company] partnership" "AI" OR "machine learning" press release
```

#### Step 3 — Multi-Source Enrichment
```
For enterprise targets, cross-reference at least 3 sources per lead:

  Source 1: LinkedIn
    → Title confirmation, tenure, reporting structure
    → Company employee count, growth rate
    → Recent posts about AI/ML topics (champion signal)

  Source 2: Company website + press
    → AI/ML team page, published case studies
    → Press releases about AI initiatives
    → Open positions on careers page

  Source 3: Community / conferences
    → Conference talks (NeurIPS, ICML, KDD, MLOps World)
    → GitHub contributions (open-source ML projects)
    → Blog posts or whitepapers on AI strategy

  MEDDIC qualification pass:
    Metrics:          "Reduced model deployment time by 60%" (from case study)
    Economic Buyer:   Chief Data Officer, reports to CEO
    Decision Criteria: SOC 2 compliance, on-prem option, Python SDK
    Decision Process:  Procurement committee, 90-day eval period
    Pain:             "Manual ML pipeline taking 3 weeks per model" (job posting)
    Champion:         Sr. ML Engineer who spoke at MLOps World about tooling gaps
```

#### Step 4 — Final Output (top 3)
| # | Name | Title | Company | Employees | Score | Qualification |
|---|------|-------|---------|-----------|-------|---------------|
| 1 | David Kim | Chief Data Officer | GlobalRetail Corp | 3,200 | 91 | MEDDIC 5/6: metrics, buyer, criteria, pain, champion |
| 2 | Lisa Zhang | VP Data Science | HealthFirst Systems | 1,800 | 86 | MEDDIC 4/6: buyer, criteria, pain, champion |
| 3 | James O'Brien | Director of AI | MegaBank Financial | 12,000 | 80 | MEDDIC 4/6: metrics, buyer, decision process, pain |

---

### Example 3: Quick-Turn SMB List Build

**Objective**: Build a 20-lead list of SMB e-commerce brands using Shopify that might need an email marketing tool. Time budget: 30 minutes.

#### Abbreviated Flow
```
ICP (quick):
  Industry: E-commerce / DTC brands
  Size: 10-100 employees
  Platform: Shopify
  Signal: Active store, social media presence, no advanced email tool detected

Search queries (5 minutes):
  site:myshopify.com "[niche]"
  "[niche] brand" "shopify" site:linkedin.com/company
  site:apps.shopify.com/reviews "[competitor email tool]" — negative reviews = opportunity
  "DTC brands" "[niche]" "founded 2022" OR "founded 2023"

Enrichment (15 minutes, per lead):
  1. Shopify store URL → active? recent products?
  2. LinkedIn company page → employee count, founded year
  3. BuiltWith → check for existing email/marketing tools
  4. Instagram/TikTok → follower count (engagement proxy)

Scoring (5 minutes):
  Use simplified scoring: ICP match (40%) + Growth signals (30%) + Reachability (30%)
  Skip MEDDIC for SMB — use BANT quick-check instead

Output (5 minutes):
  Deliver as CSV with columns: Brand, URL, Employees, Platform, Current Email Tool, Score, Contact
```

---

## Compliance & Ethics

### DO
- Use only publicly available information
- Respect robots.txt and rate limits
- Include data provenance (where each piece of info came from)
- Allow users to export and delete their lead data
- Clearly mark confidence levels on enriched data

### DO NOT
- Scrape behind login walls or paywalls
- Fabricate any lead data (even "likely" email addresses without evidence)
- Store sensitive personal data (SSN, financial info, health data)
- Send unsolicited communications on behalf of the user
- Bypass anti-scraping measures (CAPTCHAs, rate limits)
- Collect data on individuals who have opted out of data collection

### Data Retention
- Keep lead data in local files only — never exfiltrate
- Mark stale leads (>90 days without activity) for review
- Provide clear data export in all supported formats

---

## Common Pitfalls

### 1. Outdated Data
**Problem**: Company details change fast — people change jobs, startups pivot, funding info ages.
**Mitigation**:
- Verify every lead against at least 2 sources, and prefer sources updated within the last 90 days
- Flag any data point older than 6 months as "needs re-verification"
- Check LinkedIn tenure: if a contact joined their current role <3 months ago, they may not have budget authority yet

### 2. Over-Relying on a Single Source
**Problem**: Crunchbase has gaps in non-US companies. LinkedIn employee counts lag. News articles are biased toward funded companies.
**Mitigation**:
- Always cross-reference: Crunchbase funding + LinkedIn headcount + company website team page
- Use at least 2 sources for employee count (the numbers often diverge by 20-30%)
- If a company has zero press coverage, check industry-specific directories rather than discarding it

### 3. Ignoring Enrichment Quality
**Problem**: A lead list with 50 names but only 10 have titles and 5 have company size data is not actionable.
**Mitigation**:
- Set a minimum enrichment threshold before including a lead (e.g., must have: name + title + company + at least one signal)
- Track an "enrichment completeness" percentage per lead
- Return to partially-enriched leads in a second pass rather than shipping incomplete data

### 4. Vanity List Sizes
**Problem**: Delivering 100 leads when only 15 are qualified wastes the user's time and erodes trust.
**Mitigation**:
- Better to deliver 10 A-grade leads than 50 C-grade leads
- Always sort by score descending and include a clear recommendation on where to draw the cut-off line
- If the target count cannot be met at acceptable quality, say so: "Found 7 leads meeting all criteria; 13 additional leads are partial matches"

### 5. Confusing Company Name Variants
**Problem**: "Stripe, Inc.", "Stripe", and "Stripe Payments Europe Ltd" can appear as three separate leads.
**Mitigation**:
- Always normalize company names before deduplication (see Normalization Rules above)
- Match on website domain as the primary key — it is the most stable identifier
- Be especially careful with common words as company names ("Bolt", "Block", "Square")

### 6. Mistaking Hiring Activity for Purchase Intent
**Problem**: A company hiring engineers does not necessarily mean they are buying your product.
**Mitigation**:
- Hiring is a **growth signal**, not a **purchase signal** — score it accordingly (contributor, not decisive)
- Look for more direct signals: RFPs, vendor comparison blog posts, demo requests, event attendance
- Combine hiring data with tech stack analysis: hiring a "Salesforce Admin" means Salesforce budget exists

### 7. Neglecting Negative Signals
**Problem**: Focusing only on positive signals and missing red flags.
**Mitigation**:
- Check for layoffs, lawsuits, or executive departures — these reduce lead quality
- A company that just went through a 30% layoff is unlikely to approve new vendor spend
- Apply negative score modifiers:
```
Recent layoffs (>10% headcount):     -10
Lawsuit / regulatory action:         -5
Executive turnover (CEO/CTO left):   -5
Declining web traffic (per SimilarWeb): -3
```

### 8. Skipping the ICP Step
**Problem**: Jumping straight into search without a clear ICP produces scattered, low-quality results.
**Mitigation**:
- Always define the ICP **before** the first search query, even if it takes 5 extra minutes
- Write the ICP down explicitly (industry, size, geography, role, pain point, budget signal)
- Revisit and tighten the ICP after the first 10 leads if results are too broad

### Pitfall Severity Quick Reference
| Pitfall | Severity | Frequency | Fix Effort |
|---------|----------|-----------|------------|
| Outdated data | High | Very common | Medium (multi-source verification) |
| Single source reliance | High | Common | Low (add 1-2 extra sources) |
| Poor enrichment quality | Medium | Common | Medium (set thresholds, second pass) |
| Vanity list sizes | Medium | Common | Low (enforce scoring cut-off) |
| Company name variants | Medium | Very common | Low (normalize + domain match) |
| Hiring != purchase intent | Low | Occasional | Low (adjust scoring weight) |
| Ignoring negative signals | High | Common | Medium (add negative modifiers) |
| Skipping ICP | High | Occasional | Low (5-minute discipline) |
