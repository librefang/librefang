Stop two Slack formatting defects that made long replies unreadable or invisible.
Runs of blank lines now collapse to the single blank line Slack uses as a paragraph separator; the Markdown converter mapped lines one-for-one, so a model that padded its answer turned a short reply into a wall of whitespace.
The collapse runs while fenced code is masked as a single token, so code interiors keep their own blank lines.
An interactive Block Kit reply longer than Slack's 3000-character per-section limit is now split across as many sections as it needs instead of being rejected wholesale and dropped with nothing but a log line, and the section count is budgeted against the 50-blocks-per-message cap so the buttons — the functional payload — are never the thing that gets dropped.
The plain-text path was never affected: its chunker already emitted pieces under the limit (#6741) (@houko)
