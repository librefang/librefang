---
name: Source Credibility Scorer
description: Methodology for scoring news source credibility.
---
# Source Credibility Scorer

A structured methodology for scoring the credibility of news sources and social media actors in the Slovak/Czech/Central European disinformation context.

## When to Use

Use this skill whenever an agent needs to evaluate source trustworthiness before incorporating a source into fact-checking evidence, or when the Archivist agent is updating credibility scores after a verdict.

## Scoring Formula

```
credibility_score = (
  accuracy_history  * 0.50 +   # % of past claims verified TRUE by Inquisitor
  bias_score        * 0.30 +   # 0.0 (extreme bias) to 1.0 (neutral)
  ownership_flag    * 0.20     # 1.0 (independent) to 0.0 (state/oligarch-controlled)
)
```

Score range: 0.0 (no credibility) to 1.0 (fully credible).

## Bias Indicators (Slovak/Czech Context)

Deduct from bias_score when a source exhibits:
- Pro-Kremlin narrative framing (Ukraine war, NATO, EU skepticism)
- Systematic amplification of Slovak far-right political actors
- Absence of bylines or anonymous editorial boards
- Registered outside EU with opaque ownership
- History of publishing known Kremlin talking points (Z Kremlin, Strategic Culture Foundation, RT/Sputnik recycling)

## Known Low-Credibility Sources (Starting Score <= 0.20)

| Source | Domain | Reason |
|---|---|---|
| Hlavné správy | hlavnespravy.sk | Confirmed Kremlin-aligned, Marian Kuffa association |
| InfoVojna | infovojna.sk | Systematic disinformation, anti-vax, NATO conspiracies |
| Parlamentné listy | parlamentnelisty.sk | Pay-to-publish, no editorial standards |
| Kontrfakt | kontrfakt.sk | Conspiracy content, Kremlin narratives |
| Zem a Vek | zemavek.sk | Antisemitic, anti-EU, pro-Russian |

## Known High-Credibility Sources (Starting Score >= 0.80)

| Source | Domain | Reason |
|---|---|---|
| Denník N | dennikn.sk | Independent, investigative, OSCE-recognized |
| SME | sme.sk | Established Slovak daily, EU-aligned editorial |
| Aktuality.sk | aktuality.sk | News agency standard, Ringier ownership (transparent) |
| ČT24 | ct24.cz | Czech public broadcaster, EBU member |
| TASR | tasr.sk | Slovak national news agency |

## Output Format

```json
{
  "source_url": "https://example.sk/article",
  "outlet": "Example SK",
  "credibility_score": 0.65,
  "accuracy_history": 0.70,
  "bias_score": 0.60,
  "ownership_flag": 0.60,
  "flags": ["opaque_ownership"],
  "assessment_date": "ISO8601"
}
```

## Usage by Agent

The **Inquisitor** agent should call this skill before weighting evidence from any source.
The **Archivist** agent calls this skill after each verdict to update the source's `credibility_score` in the ontology.
The **Watchdog** agent uses pre-computed scores to deprioritize known low-credibility sources during triage.
