The OpenTelemetry stack moves to 0.33 as one unit: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` and `opentelemetry-http` go to 0.33 and `tracing-opentelemetry` to 0.34, with no change to how traces are exported.
Bumping any one of them alone puts two copies of `opentelemetry` in the build and breaks it, so Dependabot now groups the family into a single PR instead of one per crate.
(#8509) (@houko)
