A provider that rejects a request outright is no longer asked the same question two more times before moving on.
The retry loop treated every ambiguous HTTP failure as worth another attempt, including the ones where the provider has already judged the request itself — those cannot come out differently, since the retry sends exactly the same bytes.
It mattered most where it was least visible: history compaction handed a summarizer more text than that model could read, and the resulting refusal was retried, failed over, retried again and then repeated per chunk, leaving an agent unresponsive with no error to show for it rather than failing once and plainly.
Server-side failures keep their retries, as does a request that never finished arriving. (#8322) (@DaBlitzStein)
