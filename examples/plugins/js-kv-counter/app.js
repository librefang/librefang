/**
 * js-kv-counter — Phase-6 plugin example exercising the `kv`
 * host capability through the librefang:plugin world.
 *
 * Reads "counter" from the host KV store, increments it, and writes
 * it back. First call sets it to "1".
 *
 * Build:  cargo xtask plugins-rebuild js-kv-counter
 */
import { get, set } from 'librefang:plugin/kv@0.1.0';

/**
 * Export `run` — the world's single entry point.
 * Returning undefined maps to Ok(()); throwing maps to Err(PluginError).
 */
export function run() {
  const current = get('counter');
  const count = current !== undefined && current !== null
    ? parseInt(current, 10) + 1
    : 1;
  set('counter', String(count));
}
