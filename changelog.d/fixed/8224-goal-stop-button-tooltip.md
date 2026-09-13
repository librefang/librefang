The Goals page's Stop button says what it does.
Its tooltip used to report the run's status instead — "Running · iteration 3/10" — so the one control that stops an autonomous run described the run rather than the action, and the iteration counter it showed was already on the row beside it.
It now reads "Stop autonomous run" whether or not a run state has loaded, and the status-shaped `goals.run_active` key that fed it is gone from all five locales. (#8224) (@DaBlitzStein)
