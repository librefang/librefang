# Academic Citation Audit — Mediálny Dezolator

This document provides a systematic audit of the academic citations referenced in the repository's `README.md` and agent manifests.

## Foundation Paper

### [VERIFIED] arXiv:2508.10143
* **Citation**: Avram, A.-A., Groza, A., & Lecu, A. (2025). *MCP-Orchestrated Multi-Agent System for Automated Disinformation Detection*. 
* **Details**: Registered for the 27th International Symposium on Symbolic and Numeric Algorithms for Scientific Computing (SYNASC 2025).
* **Alignment**: Extremely high. The paper details the exact 4-agent core architecture (ML classifier, Wikipedia checker, coherence detector, and web-scraped data analyzer) orchestrated using the Model Context Protocol (MCP) to achieve cooperative disinformation detection.

---

## Wave 2 Academic Grounding

### [UNVERIFIED] Improvement 1: Bayesian weight adaptation | arXiv:2310.01555
* **Issue**: No metadata found on arXiv for this identifier.
* **Correction**: This serves as a placeholder for dynamic ensemble weighting schemes. Standard academic references for Bayesian consensus and ensemble weight adaptation include foundational machine learning papers on Bayesian Model Averaging (BMA) or online ensemble learning.

### [MISMATCHED] Improvement 2: Semantic claim deduplication | arXiv:2305.14325
* **Target Paper**: Du, Y., Li, S., Torralba, A., Tenenbaum, J. B., & Mordatch, I. (2023). *Improving Factuality and Reasoning in Language Models through Multiagent Debate*.
* **Issue**: While the paper is verified, it describes multi-agent debate (which is implemented in Wave 3, Improvement 14).
* **Correction**: SBERT-based semantic deduplication is grounded in Sentence-BERT (*Sentence-BERT: Sentence Embeddings using Siamese BERT-Networks*, Reimers & Gurevych, 2019 — `arXiv:1908.10084`).

### [UNVERIFIED] Improvement 3: Source credibility 5th ensemble signal | arXiv:2401.17786
* **Issue**: No metadata found on arXiv for this identifier.
* **Correction**: Serves as a placeholder for multi-source credibility registry integration.

### [MISMATCHED] Improvement 4: CIB detector | arXiv:2302.07934
* **Target Paper**: cosmology/physics paper concerning the *New Early Dark Energy (NEDE)* model and Hubble tension.
* **Issue**: Incorrect ID. The prompt cites Nizzoli et al. (2023), "Coordinated Inauthentic Behavior".
* **Correction**: The correct academic grounding for Coordinated Inauthentic Behavior is based on Leonardo Nizzoli's network-based coordination detection frameworks (e.g., *Coordinated Behavior on Social Media in 2019 UK General Election*, Nizzoli et al., 2021).

### [VERIFIED] Improvement 5: Aleatory/epistemic uncertainty decomposition | arXiv:2306.13063
* **Citation**: *Can LLMs Express Their Uncertainty? An Empirical Evaluation of Confidence Elicitation in LLMs* (2023).
* **Alignment**: High. Directly supports the methodologies used to elicit and decompose LLM self-confidence into aleatoric and epistemic uncertainty metrics.

### [VERIFIED] Improvement 6: Episodic memory ring buffer | arXiv:2304.03442
* **Citation**: Park, J. S., et al. (2023). *Generative Agents: Interactive Simulacra of Human Behavior*.
* **Alignment**: High. Establishes the standard architecture for LLM agent architectures utilizing memory, retrieval, reflection, and ring buffers.

### [UNVERIFIED] Improvement 7: Slovak NER entity preservation | arXiv:2305.09586
* **Issue**: No metadata found on arXiv.
* **Correction**: Serves as a placeholder for Slavic/Slovak NER pipeline optimizations.

### [VERIFIED] Improvement 8: Prompt injection shield | arXiv:2302.12173
* **Citation**: Greshake, K., et al. (2023). *More than you've asked for: A Comprehensive Analysis of Novel Prompt Injection Threats to Application-Integrated Large Language Models*.
* **Alignment**: High. Grounding research for indirect prompt injection threat detection and mitigation.

### [UNVERIFIED] Improvement 9: Cross-lingual aligner | arXiv:2209.05056
* **Issue**: No metadata found on arXiv.
* **Correction**: Serves as a placeholder for multilingual mapping and cross-lingual translation alignment (e.g., Slovak to English/Czech).

### [VERIFIED] Improvement 10: HITL confidence-gated tiers | arXiv:2308.08155
* **Citation**: Wu, Q., et al. (2023). *AutoGen: Enabling Next-Gen LLM Applications via Multi-Agent Conversation*.
* **Alignment**: High. Defines the multi-agent conversation paradigm including human-in-the-loop (HITL) gate integration.
