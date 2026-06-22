# Source Credibility Registry Provenance Chain

This document outlines the design, provenance sources, and fusion logic for the `source-rater` agent. By integrating multiple independent credibility scoring databases, the agent avoids internal bias and circular priors.

## 1. External Data Sources

We synthesize domain credibility evaluations from four independent registries:

### A. NewsGuard API
* **Provider**: NewsGuard Technologies (newsguardtech.com)
* **Type**: Professional journalistic evaluations based on 9 credibility and transparency criteria.
* **Metric**: Trust score from 0 to 100.
* **Mapping**: `newsguard_score = trust_score / 100.0`.
* **Weight**: 0.35 (reflecting high standardized auditing standards).

### B. Media Bias/Fact Check (MBFC)
* **Provider**: Media Bias/Fact Check (mediabiasfactcheck.com)
* **Type**: Independent factual reporting and bias categorization.
* **Metric**: Qualitative classifications (Very High, High, Mostly Factual, Mixed, Low, Very Low, Conspiracy/Pseudoscience).
* **Mapping**:
  * `HIGH` or `VERY HIGH` $\rightarrow$ 0.85
  * `MOSTLY FACTUAL` $\rightarrow$ 0.70
  * `MIXED` $\rightarrow$ 0.50
  * `LOW` $\rightarrow$ 0.25
  * `VERY LOW` $\rightarrow$ 0.10
  * `CONSPIRACY` / `PSEUDOSCIENCE` $\rightarrow$ 0.05
* **Weight**: 0.20.

### C. EUvsDisinfo Database
* **Provider**: European External Action Service (EEAS) East StratCom Task Force (euvsdisinfo.eu)
* **Type**: Repository of pro-Kremlin disinformation cases and outlets.
* **Metric**: Listed / Not listed as a disinformation outlet.
* **Mapping**:
  * Specifically listed $\rightarrow$ 0.10.
  * Not listed $\rightarrow$ 1.00.
* **Weight**: 0.10.

### D. Konšpirátori.sk Slovak-Specific Database
* **Provider**: Konšpirátori.sk association (konspiratori.sk)
* **Type**: Commission of Slovak journalists, academics, and experts scoring disinformation/unreliable domains in Central Europe.
* **Metric**: Rating from 1.0 to 10.0 (where higher score is riskier).
* **Mapping**: `konspiratori_score = 1.0 - (score / 10.0)`.
* **Weight**: 0.35 (reflecting localized, region-specific expertise).

---

## 2. Fusion Function

The final credibility score is a weighted average of all available ratings, preventing any single missing or biased database from dominating the evaluation.

$$credibility\_score = \frac{\sum (score_i \times weight_i)}{\sum weight_i}$$

### Insufficient Information Handling
We compute `credibility_confidence` as the number of available source registries.
* If `credibility_confidence` $< 2$, the domain lacks sufficient external consensus.
* In this case, we flag the claim for human review (`needs_human_review = true`) and default the credibility score to neutral (`0.50`).

---

## 3. Information Laundering Override

To combat the tactic of publishing narratives on low-credibility outlets before republishing them on high-credibility ones:
* If the `laundering_risk_score` $> 0.60$ (indicating the claim appeared on a known-bad source within $\le 72$ hours prior), the domain's credibility score is overridden to neutral (`0.50`) and annotated with `information_laundering_risk_override`.

---

## References
* Baly et al., "Multi-Source Fake News Classification" (arXiv:1908.05049)
* Alliance for Europe, "Information Laundering in Slovakia", March 2026.
