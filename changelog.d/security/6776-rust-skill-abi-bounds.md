The Rust WASM skill SDK now rejects negative or otherwise invalid guest-memory ranges before constructing slices, returns a null sentinel for non-positive allocations, and validates host-call response ranges against current linear memory.
Malformed ABI values can no longer create oversized or out-of-bounds Rust slices inside a skill guest (#6776) (@houko)
