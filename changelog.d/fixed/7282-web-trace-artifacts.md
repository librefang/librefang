The Web E2E workflow could never upload a Playwright trace, so every failure had to be reproduced locally before it could be read.
`playwright.config.ts` asked for `trace: 'on-first-retry'` while `retries` is `0`, so no trace file was ever written and the "Upload trace on failure" step had nothing to find.
Traces are now retained on any failure, and the upload is keyed to the Playwright step's own outcome rather than the job's, so a failure in setup or install no longer fires an upload that has no diagnostics to carry (#7282) (@houko)
