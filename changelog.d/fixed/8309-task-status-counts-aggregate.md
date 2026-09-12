`GET /api/tasks/status` now counts tasks in SQL instead of listing every row to tally four integers.
It called `task_list` with no filter, which has no `LIMIT`, and `task_prune_finished` only ever deletes completed / failed / cancelled rows — so the pending set grows for the life of the install and each poll allocated one `serde_json::Value` per task in the table before answering.
This is the endpoint the dashboard polls, which is what turned unbounded table growth into unbounded per-request heap.
The response is unchanged, including the warning logged for a status the summary does not recognise — now once per status with its count, rather than once per row. (#8309) (@houko)
