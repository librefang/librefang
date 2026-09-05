Record a version snapshot when an agent type is saved through the raw-TOML editor, which was the one write path that persisted without recording while create, the dashboard save and the history restore all recorded theirs.
Without it `GET /api/templates/{name}/history` reports the previous dashboard save as current while the file on disk is whatever the TOML tab last wrote, and a history that is silently incomplete is worse than none because nothing distinguishes the two.
The snapshot is recorded with the change source `toml` (#8028) (@DaBlitzStein)
