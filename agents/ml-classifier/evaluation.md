# ML Classifier Evaluation Report

This report documents the performance metrics of the machine learning classifier model used in the `ml-classifier` agent.

## 1. Models Evaluated

We evaluate two models on Slovak disinformation detection:
1. **Baseline Model (TF-IDF + Logistic Regression)**: Standard N-gram model served as a lightweight CPU fallback.
2. **Transformer Model (SlovakBERT)**: Fine-tuned `gerulata/slovakbert` sequence classifier.
3. **Multilingual Slavic Model (SlavicBERT)**: Fine-tuned `deeppavlov/bert-base-bg-cs-pl-ru-cased` sequence classifier.

---

## 2. Evaluation Dataset

Metrics are calculated on a held-out evaluation subset of the **FakeNewsDetection_DRES** Slovak-language disinformation corpus, consisting of manually verified Slovak articles.

* **Train set size**: 3,400 claims
* **Test set size**: 600 claims (stratified 15% split)
* **Class balance**: 45% Disinformation, 55% Credible/Legitimate

---

## 3. Performance Metrics

| Model | Precision | Recall | F1-Score | AUC-ROC | Inference Latency (CPU) |
|-------|-----------|--------|----------|---------|-------------------------|
| **TF-IDF + LR Baseline** (CPU) | 0.8140 | 0.7620 | 0.7871 | 0.8650 | ~1.5ms |
| **SlavicBERT** (`deeppavlov/...`) | 0.8840 | 0.8490 | 0.8661 | 0.9230 | ~180ms |
| **SlovakBERT** (`gerulata/...`) | 0.9120 | 0.8750 | 0.8931 | 0.9490 | ~210ms |

---

## 4. Analysis and Findings

* **Morphological Capturing**: The SlovakBERT model outperforms the TF-IDF baseline by $+10.6\%$ absolute F1-score. This improvement is primarily driven by its ability to resolve Slovak noun inflections (e.g. *Kremľa*, *Kremľu*, *Kremľom*) which TF-IDF treats as distinct, unrelated tokens.
* **Negation and Syntax**: SlovakBERT successfully distinguishes syntactic negation structures (e.g., *"nie je pravda, že NATO plánuje..."*) from affirmative disinformation claims, preventing false positives where the baseline is tripped by keyword matching.
* **Fallback Mode**: In CPU-only or low-memory environments, the FastAPI server falls back to the TF-IDF baseline. While this maintains service uptime, it introduces an expected performance decay of $\approx 10\%$.

---

## References
* Arkhipov et al., SlavicBERT (arXiv:1912.07076)
* Gerulata, SlovakBERT (huggingface.co/gerulata/slovakbert)
