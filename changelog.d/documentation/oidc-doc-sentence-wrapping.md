The OIDC role-map doc comments now follow the repository's prose convention, one sentence per line, instead of the column wrapping they landed with.
The rule exists because a hand-tuned column wrap makes a one-word edit re-flow a whole paragraph, which pollutes `git blame` and buries the real change in a review diff.
These comments were written in #7906 and so were new prose the convention applied to, not a pre-existing file exempt from it.
(#7927) (@houko)
