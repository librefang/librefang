The TUI's skill marketplace no longer lists the same skill twice.
The ClawHub index serves a skill under more than one display casing — "Prd" and "prd" come back as separate rows — and `parse_clawhub_results` passed both straight through, so the browse list showed one skill as two entries and a single install press lit the pending state on every copy of it.
Install addresses a skill by slug, so those rows are one skill by the only identity that matters here; the parser now keeps the first row for each lowercased slug, which is the spelling the index ranked highest. (@DaBlitzStein)
