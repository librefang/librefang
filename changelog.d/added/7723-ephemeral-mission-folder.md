Ephemeral workers spawned from an agent type get a uid display name and a transient mission workspace — created at spawn, deleted when the run ends, success or failure.
The folder is passed as the agent workspace root, so a worker has somewhere to drop intermediates that is guaranteed to be cleaned up rather than accumulating under the operator's home (#7860) (@DaBlitzStein)
