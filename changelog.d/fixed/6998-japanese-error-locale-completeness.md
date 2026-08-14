Added the five error-message translations missing from the Japanese Fluent locale (an agent invalid-sort key and four webhook error keys), preserving every Fluent interpolation variable used by the English source.
Added a regression test asserting the Japanese locale covers every English error key so a newly introduced key can no longer ship without a translation (#6998) (@houko)
