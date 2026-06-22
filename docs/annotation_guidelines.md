# Annotation Guidelines — Mediálny Dezolator

These guidelines are designed to ensure high inter-annotator agreement (Krippendorff's $\alpha \ge 0.65$) for human reviewers confirming or refuting disinformation verdicts. 

## 1. Satire Identification
* **Definition**: Content that uses humor, irony, exaggeration, or ridicule to expose and criticize stupidity or vices, particularly in politics.
* **Reviewer Action**: 
  * Check for satire markers (e.g. portals like *Zomri* or specific satirical columns).
  * Satire should **never** be labeled as `DISINFORMATION` unless it is framed as a serious news story by a third-party amplifier to deceive the public.
  * Classify genuine satire as `CREDIBLE` (or `UNCERTAIN` if context is highly ambiguous) with the note `"satire"`.

## 2. Opinion vs. Factual Claim
* **Definition**: Opinions are value judgments, predictions, or non-verifiable beliefs (e.g. *"Our foreign policy is a disaster"*). Factual claims assert statements about reality that can be checked (e.g. *"Slovakia sent fighter jets to Ukraine"*).
* **Reviewer Action**:
  * If a claim is purely opinion or a future prediction, it is **out of scope** for a disinformation verdict. Mark as `UNCERTAIN` or filter it out.
  * Only confirm verdicts for claims with clear factual assertions that can be proved true or false.

## 3. Partial Truths
* **Definition**: A statement that contains some element of truth but is mixed with false context, exaggeration, or omission of critical facts.
* **Reviewer Action**:
  * Determine the primary narrative impact.
  * If the false context alters the core message to mislead, label as `SUSPICIOUS` or `DISINFORMATION`.
  * If the inaccuracy is minor and non-deliberate, label as `UNCERTAIN` or `CREDIBLE` with a clarification note.

## 4. Slovak Legal Context & Defamation Law
Under Slovak law (Criminal Code, Act No. 300/2005 Coll., Section 373 - Defamation / *Ohováranie*):
* Publicly accusing a named individual or news organization of deliberate lying or spreading hostile propaganda can trigger defamation lawsuits if not backed by absolute factual proof.
* **Reviewer Action**:
  * For claims involving named individuals (politicians, journalists), ensure there are at least two independent, reputable mainstream wire services (e.g. TASR, SITA) or official government/regulatory statements corroborating the fact-check before confirming a `DISINFORMATION` verdict.
  * If evidence is conflicting, you **must** label the claim as `UNCERTAIN` or `SUSPICIOUS` to avoid legal liabilities.

---

## 5. Multiple Annotator Workflow
* For borderline claims ($P_{fake} \in [0.45, 0.75]$), at least **two independent annotators** must record a verdict.
* If annotators disagree, the claim is flagged as `CONFLATED` and sent to the chief editor for final arbitration.
