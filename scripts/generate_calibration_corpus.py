# programmatically generate calibration_corpus.json
# Author: coding-agent
# Date: 2026-06-22

import json
import os
import hashlib

def main():
    articles = []

    # 80 Legitimate journalism (known-good outlets like sme.sk, dennikn.sk, actuality.sk)
    good_domains = ["sme.sk", "dennikn.sk", "aktuality.sk", "rtvs.sk", "tasr.sk"]
    # 80 Disinformation (known-bad outlets like hlavnespravy.sk, infovojna.to, zemavek.sk)
    bad_domains = ["hlavnespravy.sk", "infovojna.to", "zemavek.sk", "bad-news-portal.sk", "slobodnyvysielac.sk"]
    # 40 Ambiguous (contested statements, political debates)
    neutral_domains = ["facebook.com", "telegram.org", "hlavnydennik.sk", "parlamentnelisty.sk"]

    # 1. 80 legitimate journalism articles
    for i in range(80):
        domain = good_domains[i % len(good_domains)]
        text = f"Slovak government announced new economic reforms to support small businesses, reported on {domain} in segment {i}."
        h = hashlib.sha256(text.encode()).hexdigest()
        articles.append({
            "_fixture": f"calibration-good-{i}",
            "source_url": f"https://www.{domain}/clanok-{i}",
            "source_domain": domain,
            "expected_verdict": "CREDIBLE",
            "claims": [
                {
                    "id": f"c{1000+i}",
                    "text": text,
                    "entities": ["Slovak government", domain],
                    "checkworthiness": 0.35,
                    "claim_hash": h
                }
            ],
            "total_sentences": 1,
            "total_claims": 1,
            "duplicates_skipped": 0,
            "extraction_timestamp": "2026-06-22T12:00:00Z"
        })

    # 2. 80 disinformation articles
    for i in range(80):
        domain = bad_domains[i % len(bad_domains)]
        text = f"SHOCKING: Secret documents reveal NATO is preparing an immediate invasion from Slovakia, published by {domain} {i}."
        h = hashlib.sha256(text.encode()).hexdigest()
        articles.append({
            "_fixture": f"calibration-bad-{i}",
            "source_url": f"https://www.{domain}/hoax-{i}",
            "source_domain": domain,
            "expected_verdict": "DISINFORMATION",
            "claims": [
                {
                    "id": f"c{2000+i}",
                    "text": text,
                    "entities": ["NATO", "Slovakia", domain],
                    "checkworthiness": 0.95,
                    "claim_hash": h
                }
            ],
            "total_sentences": 1,
            "total_claims": 1,
            "duplicates_skipped": 0,
            "extraction_timestamp": "2026-06-22T12:00:00Z"
        })

    # 3. 40 ambiguous articles
    for i in range(40):
        domain = neutral_domains[i % len(neutral_domains)]
        text = f"Contested political debate on Slovak media regulation and security cooperation with foreign allies, discussed on {domain} {i}."
        h = hashlib.sha256(text.encode()).hexdigest()
        articles.append({
            "_fixture": f"calibration-ambiguous-{i}",
            "source_url": f"https://www.{domain}/post-{i}",
            "source_domain": domain,
            "expected_verdict": "SUSPICIOUS" if i % 2 == 0 else "UNCERTAIN",
            "claims": [
                {
                    "id": f"c{3000+i}",
                    "text": text,
                    "entities": ["Slovak media", domain],
                    "checkworthiness": 0.65,
                    "claim_hash": h
                }
            ],
            "total_sentences": 1,
            "total_claims": 1,
            "duplicates_skipped": 0,
            "extraction_timestamp": "2026-06-22T12:00:00Z"
        })

    output_path = "tests/fixtures/calibration_corpus.json"
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        json.dump({"articles": articles}, f, indent=2)

    print(f"Successfully generated {len(articles)} calibration articles in {output_path}")

if __name__ == "__main__":
    main()
