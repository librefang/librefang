The agent detail view now shows what an agent actually costs to run: the tokens the daemon injects into every one of its requests, and the last five LLM calls with their input/output split and price.
Until now that number lived only in the metering tables, so the usual way to find out why one agent burns more than another was to guess at its identity files, tool list and skill registry.
The dashboard renders it in the agent drawer; the TUI puts it behind `$` on the agent detail screen.
The figures are always the open agent's own: the panel clears them when the selection moves, and an answer that arrives after the operator has moved on is discarded rather than painted under the new agent's name (#7976) (@DaBlitzStein)
