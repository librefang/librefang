Refreshing the TUI's goal list no longer discards the run state the detail fetch just retrieved.
Starting, stopping, pausing or resuming a run fires two requests on independent threads — one for the list, one for that goal's run state — and the list payload is built from stored goal documents, which never carry a phase.
Whichever landed second won, so about half the time the freshly fetched phase was overwritten with nothing, and `r` did it every time.
The visible cost was on the pause key, which reads that phase to decide between pausing and resuming and does nothing at all when it is absent: a run could be paused and then not resumed, from the same screen that was still showing it as paused.
The list now merges by goal id and keeps a phase it already knows, rather than replacing the rows wholesale. (#8224) (@DaBlitzStein)
