A sustained outage or 404 of the signed plugin registry index no longer bricks every plugin install when a previously verified copy of the index is already on disk.
When the remote fetch fails after retry, `fetch_verified_index` now serves the stale cache past its TTL, re-verifying the cached bytes against the stored Ed25519 signature with the same trust root as the remote path and logging a WARN naming the cache age.
With no verified cache on disk, installs still fail safe with the existing error.
(#8121) (@DaBlitzStein)