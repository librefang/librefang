`scripts/memory-retrieval-eval.py` compares memory retrieval strategies on your own corpus and tells you which of the differences are real.
Until now the only way to ask whether the compiled-in cosine ranking or the configured embedding model was costing a deployment anything was to copy the database out and reimplement the ranking path in a program that lived nowhere, which is why nobody had asked and why the answer changed twice when somebody finally did.
It pools every arm's results so an LLM judge cannot see which arm produced what, scores nDCG@10, and bootstraps its confidence intervals over paired per-query differences, because between-query variance dwarfs between-arm variance and an unpaired comparison cannot see the effect at all.
Judging needs a live model, so it never runs in CI; only the arithmetic is pinned, by `--self-test`.
(#7923) (@houko)
