The coverage job stops failing at the finish line.
The instrumented workspace build filled the runner's disk, so `cargo llvm-cov` ran every test to completion and then died writing the report with "No space left on device"; the job now reclaims the preinstalled SDKs it never uses before building.
(#8001) (@houko)
