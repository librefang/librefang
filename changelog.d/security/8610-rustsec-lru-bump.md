Bump `lru` 0.18.0 → 0.18.2 (RUSTSEC-2026-0253: unsound/potential use-after-free on the `LruCache::pop()` eviction path).
  `quinn-proto` 0.11.18 (RUSTSEC-2026-0338–0343) and the RUSTSEC-2024-0449 (`unsafe-libyaml`) ignore in `deny.toml` already landed on `main` independently, so this change carries only the remaining `lru` bump. (#8610) (@DaBlitzStein)
