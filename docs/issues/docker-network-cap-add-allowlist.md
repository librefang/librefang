# Docker `--network` and `--cap-add` values are not allowlisted

**Severity:** Medium
**Category:** Command injection, SSRF, sandbox
**Labels:** `security`, `sandbox`, `docker`, `medium`

## Affected files
- `crates/librefang-runtime-sandbox-docker/src/lib.rs:119-126` (`cap_add` character-set validation, but no value validation)
- `crates/librefang-runtime-sandbox-docker/src/lib.rs:134` (`config.network` passed through verbatim)

## Description

`validate_image_name` and `sanitize_container_name` are in use, but:

- `config.network` is passed through verbatim. Setting `network = "host"` makes the container share the host's network namespace → it can reach `127.0.0.1`, cloud-metadata services (`169.254.169.254`), and the daemon's own port 4545;
- `cap_add` is only character-set validated (alphanumeric + `_`), not value-validated — `SYS_ADMIN`, `NET_ADMIN`, `SYS_PTRACE`, etc. all pass.

Either one is equivalent to sandbox collapse.

## Recommendation

1. Reject `network ∈ {"host", "container:*"}`; force `bridge` / `none` / a user-defined network;
2. Maintain a safe-capability allowlist (e.g. `CHOWN`, `DAC_OVERRIDE`, `FOWNER`); anything else errors out instead of warning-and-skipping;
3. At startup, if the config contains host networking or a dangerous cap, emit `error!` and fail-fast.
