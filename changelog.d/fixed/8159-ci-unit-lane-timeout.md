Raise the `Test / Unit (lib+bin)` CI lane's `timeout-minutes` from 20 to 35, because it had outgrown the old budget and was being killed at the cap on most runs.
Measured across four consecutive `main` commits the lane took 18.7, 19.7, 20.1, 20.1 and 20.3 minutes — passing or failing by a coin flip at 94-102% of budget, with the outcome unrelated to the diff under test.
A timeout-killed job reports as `cancelled` and `CI Gate` fails on `cancelled` without re-evaluating, so each of those became a red gate that a re-run could not clear, and the failure was repeatedly misread as flaky infrastructure or as collateral from a batch merge.
The sibling `Test / Ubuntu (shard N/4)` lane already allowed 45 minutes for strictly more work, so the unit lane was under-budgeted relative to its own family rather than newly slow.
(#8159) (@houko)
