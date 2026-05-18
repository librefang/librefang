# [Medium] `whatsapp_gateway.rs` hardening — `unsafe set_var` + `JoinError` swallowed

**Severity:** Medium
**Category:** `unsafe` boundary + error handling
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description |
|--------|-------------|
| this | `spawn_blocking(\|\| unsafe { std::env::set_var(...) })` — does not acquire `secrets_env::ENV_WRITE_LOCK`, violating the reader/writer-serialization discipline required from Rust ≥ 1.74 |
| JoinError swallow | `let _ = handle.await` swallows `JoinError`: when the `spawn_blocking` task panics, the env-set silently fails and downstream WhatsApp traffic errors out with no signal |

## Affected files

- `crates/librefang-kernel/src/whatsapp_gateway.rs:184-194` (spawn + `set_var`)
- `crates/librefang-kernel/src/whatsapp_gateway.rs:187-192` (`let _ = ... .await`)
- `crates/librefang-api/src/secrets_env.rs:49-73` (`ENV_WRITE_LOCK` template)
- Similar sites to audit: `crates/librefang-api/src/routes/channels.rs:3102-3123`

## Why merged

Both items live in the same `spawn_blocking` block: one makes the env write unsafe, the other makes the failure invisible. They have to be fixed together — anything else is half a patch.

## Combined fix plan

1. Route `unsafe { set_var }` through the unified helper:
   ```rust
   librefang_api::secrets_env::set_env_var_guarded("WHATSAPP_WEB_GATEWAY_URL", url).await?;
   ```
   and lift that helper into a shared crate (so both kernel and api can use it).
2. Replace `let _ = ... .await` with an explicit match, at minimum logging `error!`:
   ```rust
   match handle.await {
       Ok(()) => {}
       Err(e) if e.is_panic() => error!(?e, "whatsapp env-set panicked"),
       Err(e) => warn!(?e, "whatsapp env-set join error"),
   }
   ```
3. Better still: inject the gateway URL via the kernel handle and remove the process-global env-write path altogether.

## Related

- The `node` launcher going through `$PATH` is a separate dimension of the same function (PATH injection) and is tracked in [docker-container-name-collisions](docker-container-name-collisions.md) (it also affects `bash` / `python` / `docker`).

## Tests

- `panic!()` inside the spawn → log shows `whatsapp env-set panicked`.
- Two concurrent paths setting env at once → `ENV_WRITE_LOCK` serializes (verify with `serial_test::serial(env)`).
