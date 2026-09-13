A database a pre-release build stamped past migration 58 now has its history tables repaired rather than merely detected.
  `CREATE TABLE IF NOT EXISTS` is a no-op against a table that exists with a different column set, so a build that created `manifest_versions` or `template_versions` with its own shape left that shape in place — and for `template_versions` there was no second chance anywhere, because the step that creates it sits below the stamp and never runs again on those machines.
  Step 60 now adds any column the code reads that the existing table lacks. (#8344) (@houko)
