The completion judge for a `loop_engineering` goal graded an empty statement for a title-only goal, since the dashboard's create form leaves the description optional and the evaluator prompt only ever carried the description.
One plausible-looking iteration was enough to draw a YES with nothing to judge it against, closing the goal on iteration one.
The judge now falls back to the title when the description is empty, matching the worker's own prompt, and skips the call entirely when both are empty rather than asking a model to grade nothing (#7785) (@DaBlitzStein)
